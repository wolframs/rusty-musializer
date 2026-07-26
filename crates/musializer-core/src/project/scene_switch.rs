//! Section-boundary scene switching plans.
//!
//! **Owner: Agent B.** Port of `../musializer/src/scene_switch.c/.h`.
//!
//! A switch plan is a **contiguous, full-duration** sequence of cues: every instant
//! of the track belongs to exactly one cue, with 1 ms of tolerated drift. That is
//! not decoration — the frame loop asks "which scene now?" and a gap would have no
//! answer.
//!
//! Every editing operation stages a private copy, mutates it, and re-publishes
//! through [`SceneSwitchTimeline::replace`], so the sorted/contiguous/coverage
//! checks run again on the result and a rejected edit leaves the timeline exactly as
//! it was. This is also the model-output staging rule in miniature: nothing mutates
//! because an operation started, only because it finished and validated.

use crate::scene::settings::{SettingsSnapshot, MAX_CONTROLS};
use crate::scene::SceneId;

/// `SCENE_SWITCH_CAPACITY` (`scene_switch.h:10`).
pub const CAPACITY: usize = 256;

/// Shortest span an editing operation will leave behind (`scene_switch.h:49`).
///
/// The coverage checks tolerate 1 ms of drift, so a cue thinner than that cannot be
/// positioned meaningfully even though `end > start` would pass. Deliberately
/// enforced only by the editing entry points: applying it in
/// [`SceneSwitchTimeline::replace`] would reject already-saved projects.
pub const MIN_CUE_SECONDS: f64 = 0.001;

/// Coverage drift the C tolerates (`scene_switch.c:55`, `:61`).
const COVERAGE_TOLERANCE: f64 = 0.001;

/// Why a switch operation failed (`Scene_Switch_Result`, `scene_switch.h:28-42`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SceneSwitchError {
    #[error("capacity violation")]
    Capacity,
    #[error("invalid duration")]
    Duration,
    #[error("invalid cue")]
    Cue,
    #[error("cues are unsorted")]
    Order,
    #[error("timeline coverage gap")]
    Coverage,
    #[error("duplicate id")]
    DuplicateId,
    #[error("no such cue")]
    Index,
    #[error("invalid cue boundary")]
    Boundary,
    #[error("settings do not match the scene")]
    Settings,
}

/// One section of the plan (`Scene_Switch_Cue`, `scene_switch.h:12-19`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSwitchCue {
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Index into the scene registry, bounded against `scene_count` on publication.
    pub scene_index: u32,
    /// How strongly the suggestion is held, `0..=1`.
    pub strength: f32,
    /// Tuning captured **from this cue's scene**. A snapshot is a bare value array
    /// whose meaning comes entirely from the scene it was captured for.
    pub settings: SettingsSnapshot,
}

impl Default for SceneSwitchCue {
    fn default() -> Self {
        Self {
            id: 0,
            start_seconds: 0.0,
            end_seconds: 0.0,
            scene_index: 0,
            strength: 0.0,
            settings: SettingsSnapshot::default(),
        }
    }
}

/// The published plan (`Scene_Switch_Timeline`, `scene_switch.h:21-26`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSwitchTimeline {
    /// The durable user opt-in, preserved across every republication.
    pub enabled: bool,
    active_index: Option<usize>,
    cues: Vec<SceneSwitchCue>,
}

impl SceneSwitchTimeline {
    /// `scene_switch_init` (`scene_switch.c:11-16`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn cues(&self) -> &[SceneSwitchCue] {
        &self.cues
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The cue [`Self::update`] last switched to, if any.
    #[must_use]
    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    /// `scene_switch_reset` (`scene_switch.c:18-21`): forgets which cue is active
    /// without touching the plan, so the next [`Self::update`] re-reports a switch.
    pub fn reset(&mut self) {
        self.active_index = None;
    }

    /// `scene_switch_replace` (`scene_switch.c:23-73`).
    ///
    /// The one way cues become published state. A zero-cue publication is refused:
    /// an empty plan cannot drive anything, and leaving the toggle on over one would
    /// strand the UI in a state where nothing switches. Use [`Self::remove`] on the
    /// last cue to clear a plan.
    pub fn replace(
        &mut self,
        cues: &[SceneSwitchCue],
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), SceneSwitchError> {
        if cues.is_empty() || cues.len() > CAPACITY {
            return Err(SceneSwitchError::Capacity);
        }
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 || scene_count == 0 {
            return Err(SceneSwitchError::Duration);
        }
        let mut cursor = 0.0f64;
        for (index, cue) in cues.iter().enumerate() {
            if cue.id == 0
                || !cue.start_seconds.is_finite()
                || !cue.end_seconds.is_finite()
                || cue.start_seconds < 0.0
                || cue.end_seconds <= cue.start_seconds
                || cue.end_seconds > duration_seconds
                || cue.scene_index >= scene_count
                || !cue.strength.is_finite()
                || !(0.0..=1.0).contains(&cue.strength)
            {
                return Err(SceneSwitchError::Cue);
            }
            // `captured` and `count` are one fact: an uncaptured snapshot has no
            // values, and a captured one has at least one.
            if cue.settings.count > MAX_CONTROLS
                || (cue.settings.captured && cue.settings.count == 0)
                || (!cue.settings.captured && cue.settings.count != 0)
                || cue.settings.values[..cue.settings.count]
                    .iter()
                    .any(|value| !value.is_finite())
            {
                return Err(SceneSwitchError::Cue);
            }
            if index > 0 && cue.start_seconds < cues[index - 1].start_seconds {
                return Err(SceneSwitchError::Order);
            }
            if (cue.start_seconds - cursor).abs() > COVERAGE_TOLERANCE {
                return Err(SceneSwitchError::Coverage);
            }
            cursor = cue.end_seconds;
            if cues[..index].iter().any(|other| other.id == cue.id) {
                return Err(SceneSwitchError::DuplicateId);
            }
        }
        if (cursor - duration_seconds).abs() > COVERAGE_TOLERANCE {
            return Err(SceneSwitchError::Coverage);
        }

        // `enabled` survives republication: it is the user's opt-in, not a property
        // of the cues.
        self.cues.clear();
        self.cues.extend_from_slice(cues);
        self.active_index = None;
        Ok(())
    }

    /// `scene_switch_next_id` (`scene_switch.c:75-85`). `None` when the id space is
    /// exhausted.
    fn next_id(&self) -> Option<u64> {
        let mut next_id = 1u64;
        for cue in &self.cues {
            if cue.id >= next_id {
                if cue.id == u64::MAX {
                    return None;
                }
                next_id = cue.id + 1;
            }
        }
        Some(next_id)
    }

    /// `scene_switch_cue_at` (`scene_switch.c:87-156`): points the plan at
    /// `scene_index` from `time_seconds` onward, splitting the covering cue when the
    /// time falls inside it.
    ///
    /// Enables the plan on success, because the user asked for a switch.
    pub fn cue_at(
        &mut self,
        time_seconds: f64,
        duration_seconds: f64,
        scene_index: u32,
        scene_count: u32,
        strength: f32,
        settings: &SettingsSnapshot,
    ) -> Result<(), SceneSwitchError> {
        if !time_seconds.is_finite()
            || !duration_seconds.is_finite()
            || duration_seconds <= 0.0
            || time_seconds < 0.0
            || time_seconds >= duration_seconds
            || scene_index >= scene_count
            || !strength.is_finite()
            || !(0.0..=1.0).contains(&strength)
            || !settings.captured
            || settings.count == 0
            || settings.count > MAX_CONTROLS
            || settings.values[..settings.count]
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(SceneSwitchError::Cue);
        }

        let mut staged = self.cues.clone();
        if staged.is_empty() {
            let id = self.next_id().ok_or(SceneSwitchError::DuplicateId)?;
            staged.push(SceneSwitchCue {
                id,
                start_seconds: 0.0,
                end_seconds: duration_seconds,
                scene_index,
                strength,
                settings: *settings,
            });
        } else {
            let selected = staged
                .iter()
                .position(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds)
                .ok_or(SceneSwitchError::Coverage)?;
            if (staged[selected].start_seconds - time_seconds).abs() <= COVERAGE_TOLERANCE {
                // Landing on a boundary retargets that cue rather than inserting a
                // hairline one next to it.
                staged[selected].scene_index = scene_index;
                staged[selected].strength = strength;
                staged[selected].settings = *settings;
            } else {
                if staged.len() >= CAPACITY {
                    return Err(SceneSwitchError::Capacity);
                }
                let id = self.next_id().ok_or(SceneSwitchError::DuplicateId)?;
                let mut next = staged[selected];
                staged[selected].end_seconds = time_seconds;
                next.id = id;
                next.start_seconds = time_seconds;
                next.scene_index = scene_index;
                next.strength = strength;
                next.settings = *settings;
                staged.insert(selected + 1, next);
            }
        }
        self.replace(&staged, duration_seconds, scene_count)?;
        self.enabled = true;
        Ok(())
    }

    /// `scene_switch_remove` (`scene_switch.c:168-199`).
    ///
    /// Drops a cue and hands its span to a neighbour, because the timeline has to
    /// stay contiguous over `[0, duration]`: the previous cue absorbs it, or the next
    /// one does when `index` is 0. Removing the only cue clears the plan **and
    /// disables it** — a timeline with zero cues cannot be published, and leaving the
    /// toggle on would strand the UI.
    pub fn remove(
        &mut self,
        index: usize,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), SceneSwitchError> {
        if index >= self.cues.len() {
            return Err(SceneSwitchError::Index);
        }
        if self.cues.len() == 1 {
            self.cues.clear();
            self.enabled = false;
            self.active_index = None;
            return Ok(());
        }
        let mut staged = self.cues.clone();
        if index == 0 {
            staged[1].start_seconds = 0.0;
        } else {
            staged[index - 1].end_seconds = staged[index].end_seconds;
        }
        staged.remove(index);
        self.replace(&staged, duration_seconds, scene_count)
    }

    /// `scene_switch_retime` (`scene_switch.c:201-224`).
    ///
    /// Moves the boundary between cue `index - 1` and cue `index`, shortening one and
    /// lengthening the other. Cue 0 begins at 0.0 by construction, so retiming it is
    /// [`SceneSwitchError::Boundary`] rather than a silent no-op. Both neighbours have
    /// to survive with a usable span, so the new boundary is bounded by the previous
    /// cue's start and this cue's end — not merely by the track duration.
    pub fn retime(
        &mut self,
        index: usize,
        start_seconds: f64,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), SceneSwitchError> {
        if index >= self.cues.len() {
            return Err(SceneSwitchError::Index);
        }
        if index == 0 {
            return Err(SceneSwitchError::Boundary);
        }
        if !start_seconds.is_finite() {
            return Err(SceneSwitchError::Cue);
        }
        let mut staged = self.cues.clone();
        if start_seconds < staged[index - 1].start_seconds + MIN_CUE_SECONDS
            || start_seconds > staged[index].end_seconds - MIN_CUE_SECONDS
        {
            return Err(SceneSwitchError::Boundary);
        }
        staged[index - 1].end_seconds = start_seconds;
        staged[index].start_seconds = start_seconds;
        self.replace(&staged, duration_seconds, scene_count)
    }

    /// `scene_switch_retarget` (`scene_switch.c:226-253`).
    ///
    /// `settings` must be a snapshot captured from the **new** scene, or `None` to
    /// clear the cue's tuning so the scene's own values apply. `None` is the safe
    /// default and the old cue's snapshot must never be carried across.
    ///
    /// This checks the snapshot against the target scene's control table, which
    /// rejects a carry between differently shaped scenes. It is **not** a complete
    /// guard: scenes 0, 1, 5, 6 and 9 all expose 8 controls, so a stale snapshot with
    /// in-range values passes and is reinterpreted control for control. Capturing
    /// from the target scene is the caller's responsibility, and this comment is the
    /// only thing standing between a future reader and that bug.
    pub fn retarget(
        &mut self,
        index: usize,
        scene_index: u32,
        settings: Option<&SettingsSnapshot>,
        duration_seconds: f64,
        scene_count: u32,
    ) -> Result<(), SceneSwitchError> {
        if index >= self.cues.len() {
            return Err(SceneSwitchError::Index);
        }
        if scene_index >= scene_count {
            return Err(SceneSwitchError::Cue);
        }
        let mut staged = self.cues.clone();
        match settings {
            None => staged[index].settings = SettingsSnapshot::default(),
            Some(settings) => {
                let scene =
                    SceneId::from_index(scene_index as usize).ok_or(SceneSwitchError::Settings)?;
                if !settings.is_valid_for(scene) || !settings.captured {
                    return Err(SceneSwitchError::Settings);
                }
                staged[index].settings = *settings;
            }
        }
        staged[index].scene_index = scene_index;
        self.replace(&staged, duration_seconds, scene_count)
    }

    /// `scene_switch_update` (`scene_switch.c:255-279`).
    ///
    /// `Ok(Some(scene_index))` means the plan just switched; `Ok(None)` is C's
    /// `NO_CHANGE` and covers three cases the caller does not need to distinguish:
    /// disabled, empty, and already on the right cue.
    pub fn update(&mut self, time_seconds: f64) -> Result<Option<u32>, SceneSwitchError> {
        if !time_seconds.is_finite() || time_seconds < 0.0 {
            return Err(SceneSwitchError::Cue);
        }
        if !self.enabled || self.cues.is_empty() {
            self.active_index = None;
            return Ok(None);
        }
        let selected = self
            .cues
            .iter()
            .position(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds);
        match selected {
            Some(selected) if Some(selected) != self.active_index => {
                self.active_index = Some(selected);
                Ok(Some(self.cues[selected].scene_index))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::{self, SceneSettings};
    use crate::scene::SCENE_COUNT;

    const SCENES: u32 = SCENE_COUNT as u32;
    const DURATION: f64 = 60.0;

    fn snapshot(scene: SceneId) -> SettingsSnapshot {
        SceneSettings::new()
            .capture(scene)
            .expect("scene defaults are in range")
    }

    fn cue(id: u64, start: f64, end: f64, scene_index: u32) -> SceneSwitchCue {
        SceneSwitchCue {
            id,
            start_seconds: start,
            end_seconds: end,
            scene_index,
            strength: 0.5,
            settings: SettingsSnapshot::default(),
        }
    }

    fn plan() -> SceneSwitchTimeline {
        let mut timeline = SceneSwitchTimeline::new();
        timeline
            .replace(
                &[
                    cue(1, 0.0, 20.0, 0),
                    cue(2, 20.0, 40.0, 1),
                    cue(3, 40.0, 60.0, 2),
                ],
                DURATION,
                SCENES,
            )
            .unwrap();
        timeline.enabled = true;
        timeline
    }

    #[test]
    fn a_plan_must_cover_the_whole_track_contiguously() {
        let mut timeline = SceneSwitchTimeline::new();
        // A gap.
        assert_eq!(
            timeline
                .replace(
                    &[cue(1, 0.0, 20.0, 0), cue(2, 30.0, 60.0, 1)],
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
        // An overlap reads as a coverage fault too: cue 2 does not begin where cue 1
        // ended.
        assert_eq!(
            timeline
                .replace(
                    &[cue(1, 0.0, 30.0, 0), cue(2, 20.0, 60.0, 1)],
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
        // Short of the end.
        assert_eq!(
            timeline
                .replace(&[cue(1, 0.0, 50.0, 0)], DURATION, SCENES)
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
        // Not starting at zero.
        assert_eq!(
            timeline
                .replace(&[cue(1, 1.0, 60.0, 0)], DURATION, SCENES)
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
        assert!(timeline.is_empty(), "a rejected plan publishes nothing");
    }

    #[test]
    fn sub_millisecond_drift_is_tolerated_and_more_is_not() {
        let mut timeline = SceneSwitchTimeline::new();
        // Half a millisecond of drift, which is what an imported analysis lane's
        // millisecond timings produce after conversion to seconds.
        assert!(timeline
            .replace(
                &[cue(1, 0.0, 20.0, 0), cue(2, 20.0005, 60.0, 1)],
                DURATION,
                SCENES
            )
            .is_ok());
        // A whole millisecond is *not* reliably tolerated, because the C test is
        // `fabs(drift) > 0.001` and `20.001 - 20.0` is `0.001000000000001` in
        // binary floating point. Reproduced rather than widened: loosening the
        // comparison here would accept plans the oracle rejects.
        assert_eq!(
            timeline
                .replace(
                    &[cue(1, 0.0, 20.0, 0), cue(2, 20.001, 60.0, 1)],
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
        assert_eq!(
            timeline
                .replace(
                    &[cue(1, 0.0, 20.0, 0), cue(2, 20.002, 60.0, 1)],
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Coverage
        );
    }

    #[test]
    fn an_empty_plan_cannot_be_published() {
        let mut timeline = SceneSwitchTimeline::new();
        assert_eq!(
            timeline.replace(&[], DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Capacity
        );
    }

    #[test]
    fn cue_bounds_and_ids_are_checked() {
        let mut timeline = SceneSwitchTimeline::new();
        let cases = [
            (cue(0, 0.0, 60.0, 0), SceneSwitchError::Cue),
            (cue(1, 0.0, 60.0, SCENES), SceneSwitchError::Cue),
            (cue(1, 0.0, 60.001, 0), SceneSwitchError::Cue),
            (cue(1, 0.0, 0.0, 0), SceneSwitchError::Cue),
            (
                SceneSwitchCue {
                    strength: 1.5,
                    ..cue(1, 0.0, 60.0, 0)
                },
                SceneSwitchError::Cue,
            ),
            (
                SceneSwitchCue {
                    strength: f32::NAN,
                    ..cue(1, 0.0, 60.0, 0)
                },
                SceneSwitchError::Cue,
            ),
            (
                SceneSwitchCue {
                    start_seconds: f64::NAN,
                    ..cue(1, 0.0, 60.0, 0)
                },
                SceneSwitchError::Cue,
            ),
        ];
        for (candidate, expected) in cases {
            assert_eq!(
                timeline
                    .replace(&[candidate], DURATION, SCENES)
                    .unwrap_err(),
                expected,
                "{candidate:?}"
            );
        }
        assert_eq!(
            timeline
                .replace(
                    &[cue(1, 0.0, 30.0, 0), cue(1, 30.0, 60.0, 1)],
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::DuplicateId
        );
        assert_eq!(
            timeline
                .replace(&[cue(1, 0.0, 60.0, 0)], 0.0, SCENES)
                .unwrap_err(),
            SceneSwitchError::Duration
        );
        assert_eq!(
            timeline
                .replace(&[cue(1, 0.0, 60.0, 0)], DURATION, 0)
                .unwrap_err(),
            SceneSwitchError::Duration
        );
    }

    #[test]
    fn a_snapshots_captured_flag_and_count_must_agree() {
        let mut timeline = SceneSwitchTimeline::new();
        let mut broken = cue(1, 0.0, 60.0, 0);
        broken.settings = SettingsSnapshot {
            captured: true,
            count: 0,
            values: [0.0; MAX_CONTROLS],
        };
        assert_eq!(
            timeline.replace(&[broken], DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Cue
        );
        broken.settings = SettingsSnapshot {
            captured: false,
            count: 2,
            values: [0.0; MAX_CONTROLS],
        };
        assert_eq!(
            timeline.replace(&[broken], DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Cue
        );
        broken.settings = SettingsSnapshot {
            captured: true,
            count: 1,
            values: [f32::NAN; MAX_CONTROLS],
        };
        assert_eq!(
            timeline.replace(&[broken], DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Cue
        );
    }

    #[test]
    fn publication_preserves_the_user_opt_in() {
        let mut timeline = plan();
        assert!(timeline.enabled);
        timeline
            .replace(&[cue(9, 0.0, 60.0, 3)], DURATION, SCENES)
            .unwrap();
        assert!(
            timeline.enabled,
            "the toggle is the user's, not a property of the cues"
        );
        assert_eq!(
            timeline.active_index(),
            None,
            "and the active cue is forgotten"
        );
    }

    #[test]
    fn update_reports_a_switch_once_per_cue() {
        let mut timeline = plan();
        assert_eq!(timeline.update(0.0).unwrap(), Some(0));
        assert_eq!(timeline.update(5.0).unwrap(), None, "same cue, no change");
        assert_eq!(timeline.update(25.0).unwrap(), Some(1));
        assert_eq!(timeline.update(45.0).unwrap(), Some(2));
        assert_eq!(timeline.update(59.9).unwrap(), None);
        // Past the end there is no covering cue, so nothing changes.
        assert_eq!(timeline.update(60.0).unwrap(), None);
        assert_eq!(timeline.update(-1.0).unwrap_err(), SceneSwitchError::Cue);
        assert_eq!(
            timeline.update(f64::NAN).unwrap_err(),
            SceneSwitchError::Cue
        );
    }

    #[test]
    fn a_disabled_plan_never_switches() {
        let mut timeline = plan();
        timeline.enabled = false;
        assert_eq!(timeline.update(25.0).unwrap(), None);
        assert_eq!(timeline.active_index(), None);
    }

    #[test]
    fn reset_makes_the_next_update_report_again() {
        let mut timeline = plan();
        assert_eq!(timeline.update(25.0).unwrap(), Some(1));
        assert_eq!(timeline.update(25.0).unwrap(), None);
        timeline.reset();
        assert_eq!(timeline.update(25.0).unwrap(), Some(1));
    }

    #[test]
    fn cue_at_splits_the_covering_cue_and_enables_the_plan() {
        let mut timeline = SceneSwitchTimeline::new();
        timeline
            .replace(&[cue(1, 0.0, 60.0, 0)], DURATION, SCENES)
            .unwrap();
        assert!(!timeline.enabled);
        timeline
            .cue_at(
                30.0,
                DURATION,
                4,
                SCENES,
                0.8,
                &snapshot(SceneId::SongAtlas),
            )
            .unwrap();
        assert!(timeline.enabled, "asking for a switch turns the plan on");
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline.cues()[0].end_seconds, 30.0);
        assert_eq!(timeline.cues()[1].start_seconds, 30.0);
        assert_eq!(timeline.cues()[1].end_seconds, 60.0);
        assert_eq!(timeline.cues()[1].scene_index, 4);
        assert_ne!(timeline.cues()[0].id, timeline.cues()[1].id);
    }

    #[test]
    fn cue_at_on_a_boundary_retargets_rather_than_inserting() {
        let mut timeline = plan();
        let before = timeline.len();
        timeline
            .cue_at(
                20.0,
                DURATION,
                5,
                SCENES,
                0.25,
                &snapshot(SceneId::SpectralTerrarium),
            )
            .unwrap();
        assert_eq!(timeline.len(), before, "no hairline cue is created");
        assert_eq!(timeline.cues()[1].scene_index, 5);
        assert_eq!(timeline.cues()[1].strength, 0.25);
    }

    #[test]
    fn cue_at_on_an_empty_plan_covers_the_whole_track() {
        let mut timeline = SceneSwitchTimeline::new();
        timeline
            .cue_at(
                10.0,
                DURATION,
                2,
                SCENES,
                1.0,
                &snapshot(SceneId::OrbitalLattice),
            )
            .unwrap();
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline.cues()[0].start_seconds, 0.0);
        assert_eq!(timeline.cues()[0].end_seconds, DURATION);
    }

    #[test]
    fn cue_at_refuses_arguments_that_cannot_produce_a_valid_plan() {
        let mut timeline = plan();
        let before = timeline.clone();
        let captured = snapshot(SceneId::Spectrum);
        for (time, scene_index, strength, settings) in [
            (60.0, 0, 0.5f32, captured),
            (-1.0, 0, 0.5, captured),
            (f64::NAN, 0, 0.5, captured),
            (10.0, SCENES, 0.5, captured),
            (10.0, 0, 1.5, captured),
            (10.0, 0, f32::NAN, captured),
            (10.0, 0, 0.5, SettingsSnapshot::default()),
        ] {
            assert_eq!(
                timeline
                    .cue_at(time, DURATION, scene_index, SCENES, strength, &settings)
                    .unwrap_err(),
                SceneSwitchError::Cue
            );
            assert_eq!(timeline, before, "a rejected edit changes nothing");
        }
    }

    #[test]
    fn remove_hands_the_span_to_a_neighbour() {
        let mut timeline = plan();
        timeline.remove(1, DURATION, SCENES).unwrap();
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline.cues()[0].start_seconds, 0.0);
        assert_eq!(
            timeline.cues()[0].end_seconds,
            40.0,
            "the previous cue absorbs it"
        );
        assert_eq!(timeline.cues()[1].start_seconds, 40.0);

        // Removing the first cue makes the next one start at zero instead.
        let mut timeline = plan();
        timeline.remove(0, DURATION, SCENES).unwrap();
        assert_eq!(timeline.cues()[0].start_seconds, 0.0);
        assert_eq!(timeline.cues()[0].end_seconds, 40.0);
    }

    #[test]
    fn removing_the_last_cue_clears_and_disables_the_plan() {
        let mut timeline = SceneSwitchTimeline::new();
        timeline
            .replace(&[cue(1, 0.0, 60.0, 0)], DURATION, SCENES)
            .unwrap();
        timeline.enabled = true;
        timeline.remove(0, DURATION, SCENES).unwrap();
        assert!(timeline.is_empty());
        assert!(
            !timeline.enabled,
            "an enabled plan with nothing in it would strand the UI"
        );
        assert_eq!(timeline.active_index(), None);
    }

    #[test]
    fn remove_refuses_an_index_that_is_not_there() {
        let mut timeline = plan();
        assert_eq!(
            timeline.remove(3, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Index
        );
        assert_eq!(timeline.len(), 3);
    }

    #[test]
    fn retime_moves_one_boundary_and_keeps_both_neighbours_usable() {
        let mut timeline = plan();
        timeline.retime(1, 25.0, DURATION, SCENES).unwrap();
        assert_eq!(timeline.cues()[0].end_seconds, 25.0);
        assert_eq!(timeline.cues()[1].start_seconds, 25.0);
        assert_eq!(timeline.cues()[1].end_seconds, 40.0);
    }

    #[test]
    fn retime_refuses_to_squeeze_a_neighbour_out_of_existence() {
        let mut timeline = plan();
        let before = timeline.clone();
        // Cue 0 starts at 0.0 by construction, so its boundary is not movable.
        assert_eq!(
            timeline.retime(0, 5.0, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Boundary
        );
        // Onto the previous cue's own start, and onto this cue's end.
        assert_eq!(
            timeline.retime(1, 0.0, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Boundary
        );
        assert_eq!(
            timeline.retime(1, 40.0, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Boundary
        );
        assert_eq!(
            timeline.retime(1, f64::NAN, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Cue
        );
        assert_eq!(
            timeline.retime(9, 25.0, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Index
        );
        assert_eq!(timeline, before);
        // Exactly one millisecond of span on each side is the documented floor.
        let mut timeline = plan();
        assert!(timeline
            .retime(1, 0.0 + MIN_CUE_SECONDS, DURATION, SCENES)
            .is_ok());
    }

    #[test]
    fn retarget_clears_tuning_by_default() {
        let mut timeline = plan();
        timeline
            .retarget(1, 7, None, DURATION, SCENES)
            .expect("clearing tuning is always safe");
        assert_eq!(timeline.cues()[1].scene_index, 7);
        assert!(!timeline.cues()[1].settings.captured);
        assert_eq!(timeline.cues()[1].settings.count, 0);
    }

    #[test]
    fn retarget_rejects_a_snapshot_shaped_for_a_different_scene() {
        let mut timeline = plan();
        // Song Atlas has 12 controls; Cadence has 7, so carrying that snapshot
        // across is caught.
        let atlas = snapshot(SceneId::SongAtlas);
        assert_eq!(settings::count(SceneId::SongAtlas), 12);
        assert_eq!(settings::count(SceneId::Cadence), 7);
        assert_eq!(
            timeline
                .retarget(
                    1,
                    SceneId::Cadence.index() as u32,
                    Some(&atlas),
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Settings
        );
        // The matching snapshot is accepted.
        let cadence = snapshot(SceneId::Cadence);
        assert!(timeline
            .retarget(
                1,
                SceneId::Cadence.index() as u32,
                Some(&cadence),
                DURATION,
                SCENES
            )
            .is_ok());
    }

    #[test]
    fn retarget_cannot_always_catch_a_carry_between_same_shaped_scenes() {
        // This documents a known hole rather than asserting desirable behaviour.
        // Scenes 0, 1, 5, 6 and 9 all expose 8 controls, so the count check cannot
        // tell them apart, and whenever the values also happen to be in range for
        // the target the stale snapshot is accepted and reinterpreted control for
        // control. Capturing from the target scene is the caller's job; the header
        // says so and so does this test.
        let mut same_shape_pairs = 0usize;
        let mut carried_silently = 0usize;
        for source in SceneId::ALL {
            for target in SceneId::ALL {
                if source == target || settings::count(source) != settings::count(target) {
                    continue;
                }
                same_shape_pairs += 1;
                let mut timeline = plan();
                if timeline
                    .retarget(
                        1,
                        target.index() as u32,
                        Some(&snapshot(source)),
                        DURATION,
                        SCENES,
                    )
                    .is_ok()
                {
                    carried_silently += 1;
                }
            }
        }
        assert!(same_shape_pairs > 0, "the registry has same-shaped scenes");
        assert!(
            carried_silently > 0,
            "at least one carry slips through; the value-range check is not a \
             substitute for capturing from the target scene"
        );
    }

    #[test]
    fn retarget_refuses_an_unknown_scene_or_index() {
        let mut timeline = plan();
        assert_eq!(
            timeline.retarget(9, 1, None, DURATION, SCENES).unwrap_err(),
            SceneSwitchError::Index
        );
        assert_eq!(
            timeline
                .retarget(1, SCENES, None, DURATION, SCENES)
                .unwrap_err(),
            SceneSwitchError::Cue
        );
        // A scene_count larger than the registry means an index with no scene, and
        // the snapshot check is what notices.
        assert_eq!(
            timeline
                .retarget(
                    1,
                    SCENES,
                    Some(&snapshot(SceneId::Spectrum)),
                    DURATION,
                    SCENES + 1
                )
                .unwrap_err(),
            SceneSwitchError::Settings
        );
    }
}
