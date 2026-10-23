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

use super::look::{Finish, Grade};
use super::pixel::{Pixel, PixelFrame};
use super::transition::{self, Active, Section, Transition};
use crate::scene::settings::{SceneSettings, SettingsSnapshot, MAX_CONTROLS};
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

/// Longest glide a section may ask for, in seconds (post-legacy, MV9-c).
pub const MAX_GLIDE_SECONDS: f32 = 30.0;

/// Whether a glide is a value a plan may hold: zero (none), or a finite length
/// up to [`MAX_GLIDE_SECONDS`].
#[must_use]
pub fn glide_is_valid(seconds: f32) -> bool {
    seconds.is_finite() && (0.0..=MAX_GLIDE_SECONDS).contains(&seconds)
}

/// A whole-number control with fewer steps than this across its range is a
/// choice and snaps during a glide; see [`SceneSwitchTimeline::glided_settings`].
pub const GLIDE_CHOICE_STEPS: f32 = 16.0;

fn glides(descriptor: &crate::scene::SettingDescriptor) -> bool {
    use crate::scene::SettingKind;
    match descriptor.kind {
        SettingKind::Toggle => false,
        SettingKind::Slider => {
            descriptor.precision > 0
                || descriptor.maximum - descriptor.minimum >= GLIDE_CHOICE_STEPS
        }
    }
}

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
    /// How this section arrives (post-legacy, MV3). A cut by default, which is
    /// the C's plan and what nothing serializes.
    pub transition: Transition,
    /// The palette this section is held to (post-legacy, MV9-b).
    pub grade: Option<Grade>,
    /// Seconds over which this section's tuning eases in from the section
    /// before it, when both are the same scene (post-legacy, MV9-c).
    pub glide: f32,
    /// This section's pixel look, over the finish's (2026-10-24): `None`
    /// wears the video's, `Some(Pixel::OFF)` switches it off here.
    pub pixel: Option<Pixel>,
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
            transition: Transition::CUT,
            grade: None,
            glide: 0.0,
            pixel: None,
        }
    }
}

/// The published plan (`Scene_Switch_Timeline`, `scene_switch.h:21-26`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneSwitchTimeline {
    /// The durable user opt-in, preserved across every republication.
    pub enabled: bool,
    /// How the track ends (post-legacy, MV3): its kind played backwards into
    /// black over the final seconds. Independent of the cues, so a track with
    /// no plan can still end softly.
    pub outro: Transition,
    /// The finish over every frame of the track (post-legacy, MV9-a). Like the
    /// outro it is the track's own, so it applies whether or not the plan is
    /// switched on.
    pub finish: Finish,
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

    /// Replaces the active cue's captured settings without rewinding playback
    /// (`commit_active_cue_settings`, `plug.c:1043-1052`).
    ///
    /// Unlike [`Self::replace`], this is an edit to the cue already playing, so
    /// resetting `active_index` would make the same boundary fire again on the
    /// next frame.
    pub fn set_active_settings(&mut self, scene: SceneId, settings: &SettingsSnapshot) -> bool {
        let Some(active) = self.active_index else {
            return false;
        };
        let Some(cue) = self.cues.get_mut(active) else {
            return false;
        };
        if cue.scene_index != scene.index() as u32
            || !settings.captured
            || !settings.is_valid_for(scene)
        {
            return false;
        }
        cue.settings = *settings;
        true
    }

    /// The transition in force at `time_seconds`, if any. `None` while the
    /// plan is switched off, except for the outro, which is the track's own.
    #[must_use]
    pub fn transition_at(&self, time_seconds: f64, duration_seconds: f64) -> Option<Active> {
        let sections = self.sections();
        transition::resolve(&sections, self.outro, duration_seconds, time_seconds)
    }

    /// The enabled plan's boundaries, as [`transition::resolve`] reads them.
    #[must_use]
    pub fn sections(&self) -> Vec<Section> {
        if !self.enabled {
            return Vec::new();
        }
        self.cues
            .iter()
            .map(|cue| Section {
                id: cue.id,
                start_seconds: cue.start_seconds,
                end_seconds: cue.end_seconds,
                transition: cue.transition,
            })
            .collect()
    }

    /// The cue covering `time_seconds`, while the plan is on.
    #[must_use]
    pub fn cue_covering(&self, time_seconds: f64) -> Option<&SceneSwitchCue> {
        if !self.enabled {
            return None;
        }
        self.cues
            .iter()
            .find(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds)
            .or_else(|| {
                // The last frame of the track sits exactly on the last cue's end.
                self.cues.last().filter(|cue| {
                    time_seconds >= cue.start_seconds && time_seconds <= cue.end_seconds
                })
            })
    }

    /// The palette lock in force at `time_seconds` (MV9-b): the covering
    /// section's, while the plan is on.
    #[must_use]
    pub fn grade_at(&self, time_seconds: f64) -> Option<Grade> {
        self.cue_covering(time_seconds)
            .and_then(|cue| cue.grade)
            .filter(|grade| !grade.is_identity())
    }

    /// The grade a scene wears near `time_seconds` (MV9-d): that of the
    /// section of `scene` nearest in time, while the plan is on.
    ///
    /// For a scene shown *inside* another — the threshold's doorway — which has
    /// no section of its own at that moment. Wearing the look its own nearest
    /// section wears is what makes the cut from the doorway into that section
    /// seamless instead of a palette jump.
    #[must_use]
    pub fn grade_near(&self, time_seconds: f64, scene: SceneId) -> Option<Grade> {
        self.cue_near(time_seconds, scene)
            .and_then(|cue| cue.grade)
            .filter(|grade| !grade.is_identity())
    }

    /// The pixel look in force at `time_seconds` (2026-10-24): the covering
    /// section's own, else the finish's.
    ///
    /// When a section's look differs from the one before it, the two
    /// dissolve cell by cell in Bayer order — how a cartridge cross-fades —
    /// across the section's **transition**, or, when it arrives by a cut,
    /// across its **glide**. The glide is the one to reach for when the scene
    /// carries on: two sections of one scene, the second with `"levels"` and
    /// a glide of 0.9, unlock a four-shade world into colour over a live
    /// picture, where a transition would dissolve out of a held still. With
    /// neither it snaps, which is what a drop wants. A `dip` hides the change
    /// in its black, so it snaps there too. Pure arithmetic over the
    /// playhead, so a preview, a still and an export agree.
    #[must_use]
    pub fn pixel_at(&self, time_seconds: f64) -> PixelFrame {
        let base = self.finish.pixel;
        if !self.enabled {
            return PixelFrame::steady(base);
        }
        let Some(index) = self.covering_index(time_seconds) else {
            return PixelFrame::steady(base);
        };
        let cue = &self.cues[index];
        let to = cue.pixel.unwrap_or(base);
        if index == 0 || cue.transition.kind == super::transition::TransitionKind::Dip {
            return PixelFrame::steady(to);
        }
        let from = self.cues[index - 1].pixel.unwrap_or(base);
        let room = cue.end_seconds - cue.start_seconds;
        let length = if cue.transition.is_cut() {
            f64::from(cue.glide).min(room)
        } else {
            transition::fitted_seconds(cue.transition, room)
        };
        let elapsed = time_seconds - cue.start_seconds;
        if from == to || length <= 0.0 || elapsed >= length {
            return PixelFrame::steady(to);
        }
        PixelFrame {
            from,
            to,
            mix: (elapsed / length).clamp(0.0, 1.0) as f32,
        }
    }

    /// The index of the cue covering `time_seconds`, while the plan is on —
    /// [`Self::cue_covering`]'s rule, including the last frame.
    fn covering_index(&self, time_seconds: f64) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        self.cues
            .iter()
            .position(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds)
            .or_else(|| {
                let last = self.cues.len().checked_sub(1)?;
                let cue = &self.cues[last];
                (time_seconds >= cue.start_seconds && time_seconds <= cue.end_seconds)
                    .then_some(last)
            })
    }

    /// Sets one section's pixel look (`None` wears the video's). Refuses an
    /// unknown id or a look outside its bounds, and changes nothing when it
    /// refuses.
    pub fn set_pixel(&mut self, cue_id: u64, pixel: Option<Pixel>) -> bool {
        if pixel.is_some_and(|pixel| !pixel.is_valid()) {
            return false;
        }
        match self.cues.iter_mut().find(|cue| cue.id == cue_id) {
            Some(cue) => {
                cue.pixel = pixel;
                true
            }
            None => false,
        }
    }

    /// The section of `scene` nearest in time to `time_seconds`, while the
    /// plan is on (MV9-d/j): what a scene shown inside the threshold's
    /// doorway takes its tuning and its grade from, having no section of its
    /// own at that moment.
    #[must_use]
    pub fn cue_near(&self, time_seconds: f64, scene: SceneId) -> Option<&SceneSwitchCue> {
        if !self.enabled {
            return None;
        }
        let index = scene.index() as u32;
        self.cues
            .iter()
            .filter(|cue| cue.scene_index == index)
            .min_by(|a, b| {
                let distance = |cue: &SceneSwitchCue| {
                    if time_seconds < cue.start_seconds {
                        cue.start_seconds - time_seconds
                    } else if time_seconds >= cue.end_seconds {
                        time_seconds - cue.end_seconds
                    } else {
                        0.0
                    }
                };
                distance(a).total_cmp(&distance(b))
            })
    }

    /// The tuning in force while a section glides in (MV9-c), or `None` when
    /// nothing is gliding at `time_seconds`.
    ///
    /// `base` is the track's per-scene table, the one a cue's snapshot is
    /// applied over. A section glides only from a section **of the same
    /// scene** directly before it — a different scene starts fresh, so there
    /// is nothing to ease from. The ease is a smoothstep over `glide` seconds
    /// from the section's start.
    ///
    /// Not every control eases. A toggle snaps, and so does a whole-number
    /// control whose range spans fewer than [`GLIDE_CHOICE_STEPS`] steps: that
    /// is a *choice* (Stage's `layout`, Phosphor's `field`, a cat count), and
    /// passing through the choices between two of them would show pictures
    /// nobody asked for. A whole-number control with a wide range — a hue in
    /// degrees, a ring count — is a quantity, and eases.
    #[must_use]
    pub fn glided_settings(
        &self,
        base: &SceneSettings,
        time_seconds: f64,
    ) -> Option<SceneSettings> {
        if !self.enabled {
            return None;
        }
        let index = self
            .cues
            .iter()
            .position(|cue| cue.start_seconds <= time_seconds && time_seconds < cue.end_seconds)?;
        let cue = &self.cues[index];
        if index == 0 || cue.glide <= 0.0 {
            return None;
        }
        let elapsed = time_seconds - cue.start_seconds;
        if elapsed >= f64::from(cue.glide) {
            return None;
        }
        let previous = &self.cues[index - 1];
        if previous.scene_index != cue.scene_index {
            return None;
        }
        let scene = SceneId::from_index(cue.scene_index as usize)?;
        let resolve = |snapshot: &SettingsSnapshot| {
            let mut table = *base;
            if snapshot.captured {
                table.apply_snapshot(scene, snapshot);
            }
            table
        };
        let from = resolve(&previous.settings);
        let mut to = resolve(&cue.settings);
        let t = (elapsed / f64::from(cue.glide)).clamp(0.0, 1.0) as f32;
        let k = t * t * (3.0 - 2.0 * t);
        for (control, descriptor) in crate::scene::settings::descriptors(scene)
            .iter()
            .enumerate()
        {
            if !glides(descriptor) {
                continue;
            }
            let a = from.get(scene, control);
            let b = to.get(scene, control);
            to.set(scene, control, a + (b - a) * k);
        }
        Some(to)
    }

    /// Sets how one section arrives. Refuses an unknown id or a transition
    /// outside its bounds, and changes nothing when it refuses.
    pub fn set_transition(&mut self, cue_id: u64, transition: Transition) -> bool {
        if !transition.is_valid() {
            return false;
        }
        match self.cues.iter_mut().find(|cue| cue.id == cue_id) {
            Some(cue) => {
                cue.transition = transition;
                true
            }
            None => false,
        }
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
                || !cue.transition.is_valid()
                || cue.grade.is_some_and(|grade| !grade.is_valid())
                || !glide_is_valid(cue.glide)
                || cue.pixel.is_some_and(|pixel| !pixel.is_valid())
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
                transition: crate::project::transition::Transition::CUT,
                grade: None,
                glide: 0.0,
                pixel: None,
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
            transition: Transition::CUT,
            grade: None,
            glide: 0.0,
            pixel: None,
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
    fn a_section_glides_in_from_the_same_scene_before_it_and_choices_snap() {
        use crate::scene::settings::index::stage;
        let scene = SceneId::Stage;
        let mut first = SceneSettings::new();
        first.set(scene, stage::LAYOUT, 0.0);
        first.set(scene, stage::HUE, -100.0);
        first.set(scene, stage::SCALE, 0.6);
        first.set(scene, stage::FAMILIAR, 0.0);
        let mut second = first;
        second.set(scene, stage::LAYOUT, 4.0);
        second.set(scene, stage::HUE, 100.0);
        second.set(scene, stage::SCALE, 1.4);
        second.set(scene, stage::FAMILIAR, 1.0);
        let index = scene.index() as u32;
        let mut a = cue(1, 0.0, 20.0, index);
        a.settings = first.capture(scene).unwrap();
        let mut b = cue(2, 20.0, 40.0, index);
        b.settings = second.capture(scene).unwrap();
        b.glide = 4.0;
        let c = cue(3, 40.0, 60.0, SceneId::Loom.index() as u32);
        let mut timeline = SceneSwitchTimeline::new();
        timeline.replace(&[a, b, c], DURATION, SCENES).unwrap();
        timeline.enabled = true;
        let base = SceneSettings::new();

        assert!(
            timeline.glided_settings(&base, 10.0).is_none(),
            "not gliding"
        );
        assert!(
            timeline.glided_settings(&base, 24.0).is_none(),
            "glide over"
        );
        let mid = timeline.glided_settings(&base, 22.0).expect("halfway in");
        // Smoothstep at one half is one half.
        assert!((mid.get(scene, stage::SCALE) - 1.0).abs() < 1.0e-5);
        assert!(
            mid.get(scene, stage::HUE).abs() < 1.0e-3,
            "a hue in degrees eases"
        );
        assert_eq!(
            mid.get(scene, stage::LAYOUT),
            4.0,
            "a layout is a choice and snaps"
        );
        assert_eq!(mid.get(scene, stage::FAMILIAR), 1.0, "a toggle snaps");
        let start = timeline.glided_settings(&base, 20.0).unwrap();
        assert!(
            (start.get(scene, stage::SCALE) - 0.6).abs() < 1.0e-5,
            "continuous at the cut"
        );

        // A different scene before it: nothing to ease from.
        let mut d = cue(3, 40.0, 60.0, SceneId::Loom.index() as u32);
        d.glide = 4.0;
        let mut other = SceneSwitchTimeline::new();
        other
            .replace(&[a, cue(2, 20.0, 40.0, index), d], DURATION, SCENES)
            .unwrap();
        other.enabled = true;
        assert!(other.glided_settings(&base, 41.0).is_none());

        // A plan switched off glides nothing.
        timeline.enabled = false;
        assert!(timeline.glided_settings(&base, 22.0).is_none());
    }

    #[test]
    fn a_nested_scene_wears_its_nearest_sections_grade() {
        use crate::project::look::{Grade, Palette};
        let threshold = SceneId::Threshold.index() as u32;
        let constellation = SceneId::Constellation.index() as u32;
        let a = cue(1, 0.0, 20.0, threshold);
        let mut b = cue(2, 20.0, 40.0, constellation);
        b.grade = Some(Grade::from_palette(Palette::NightDoor, 0.8));
        let c = cue(3, 40.0, 50.0, threshold);
        let mut e = cue(4, 50.0, 60.0, constellation);
        e.grade = Some(Grade::from_palette(Palette::Ember, 0.8));
        let mut timeline = SceneSwitchTimeline::new();
        timeline.replace(&[a, b, c, e], DURATION, SCENES).unwrap();
        timeline.enabled = true;
        assert_eq!(timeline.grade_near(5.0, SceneId::Constellation), b.grade);
        assert_eq!(timeline.grade_near(42.0, SceneId::Constellation), b.grade);
        assert_eq!(timeline.grade_near(48.0, SceneId::Constellation), e.grade);
        assert_eq!(timeline.grade_near(5.0, SceneId::Loom), None);
    }

    #[test]
    fn the_pixel_look_is_the_sections_else_the_finishs_and_dissolves_on_a_transition() {
        use crate::project::pixel::{Matching, Pixel, PixelPalette};
        use crate::project::transition::{Transition, TransitionKind};
        let green = Pixel::on();
        let colour = Pixel::on().without_colours(Matching::None);
        let a = cue(1, 0.0, 20.0, 0);
        let mut b = cue(2, 20.0, 40.0, 0);
        b.pixel = Some(colour);
        b.transition = Transition::new(TransitionKind::Fade, 2.0);
        let mut c = cue(3, 40.0, 50.0, 0);
        c.pixel = Some(Pixel::OFF);
        let mut d = cue(4, 50.0, 55.0, 0);
        d.transition = Transition::new(TransitionKind::Dip, 2.0);
        // Same scene, a cut and a glide: the look dissolves over a live
        // picture.
        let mut e = cue(5, 55.0, 60.0, 0);
        e.pixel = Some(colour);
        e.glide = 1.0;
        let mut timeline = SceneSwitchTimeline::new();
        timeline
            .replace(&[a, b, c, d, e], DURATION, SCENES)
            .unwrap();
        timeline.finish.pixel = green;

        // A plan switched off wears the finish's, whatever its sections say.
        assert_eq!(timeline.pixel_at(45.0), PixelFrame::steady(green));
        timeline.enabled = true;
        assert_eq!(timeline.pixel_at(10.0), PixelFrame::steady(green));
        // The unlock: half way through the fade, half the cells are colour.
        let unlocking = timeline.pixel_at(21.0);
        assert_eq!((unlocking.from, unlocking.to), (green, colour));
        assert!((unlocking.mix - 0.5).abs() < 1.0e-6);
        assert_eq!(timeline.pixel_at(23.0), PixelFrame::steady(colour));
        // A cut snaps, and `off` is a look a section may ask for.
        assert_eq!(timeline.pixel_at(40.0), PixelFrame::steady(Pixel::OFF));
        assert!(!timeline.pixel_at(41.0).is_on());
        // A dip hides the change in its black.
        assert_eq!(timeline.pixel_at(50.5), PixelFrame::steady(green));
        // A glide dissolves on a cut.
        let gliding = timeline.pixel_at(55.25);
        assert_eq!((gliding.from, gliding.to), (green, colour));
        assert!((gliding.mix - 0.25).abs() < 1.0e-6);
        assert_eq!(timeline.pixel_at(DURATION), PixelFrame::steady(colour));

        // Editing one section's look validates it and leaves the rest.
        assert!(timeline.set_pixel(4, Some(green.with_palette(PixelPalette::Grey))));
        assert!(!timeline.set_pixel(4, Some(Pixel { pixels: 9, ..green })));
        assert!(!timeline.set_pixel(99, None));
        assert_eq!(
            timeline.cues()[3].pixel.map(|pixel| pixel.named_palette()),
            Some(Some(PixelPalette::Grey))
        );
        let mut bad = cue(1, 0.0, DURATION, 0);
        bad.pixel = Some(Pixel {
            dither: 2.0,
            ..green
        });
        assert_eq!(
            SceneSwitchTimeline::new().replace(&[bad], DURATION, SCENES),
            Err(SceneSwitchError::Cue)
        );
    }

    #[test]
    fn the_grade_in_force_is_the_covering_sections() {
        use crate::project::look::{Grade, Palette};
        let mut a = cue(1, 0.0, 30.0, 0);
        a.grade = Some(Grade::from_palette(Palette::Ember, 0.5));
        let mut b = cue(2, 30.0, 60.0, 0);
        b.grade = Some(Grade::from_palette(Palette::Mono, 0.0));
        let mut timeline = SceneSwitchTimeline::new();
        timeline.replace(&[a, b], DURATION, SCENES).unwrap();
        timeline.enabled = true;
        assert_eq!(timeline.grade_at(10.0), a.grade);
        assert_eq!(
            timeline.grade_at(40.0),
            None,
            "an identity grade is no grade"
        );
        assert_eq!(
            timeline.grade_at(DURATION),
            None,
            "the last frame is the last cue's"
        );
        let mut bad = a;
        bad.grade = Some(Grade {
            map: 2.0,
            ..Grade::from_palette(Palette::Ember, 0.5)
        });
        assert_eq!(
            timeline.replace(&[bad, b], DURATION, SCENES),
            Err(SceneSwitchError::Cue)
        );
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
        // Song Atlas has 12 controls; Loom has 7, so carrying that snapshot
        // across is caught.
        let atlas = snapshot(SceneId::SongAtlas);
        assert_eq!(settings::count(SceneId::SongAtlas), 12);
        assert_eq!(settings::count(SceneId::Loom), 7);
        assert_eq!(
            timeline
                .retarget(
                    1,
                    SceneId::Loom.index() as u32,
                    Some(&atlas),
                    DURATION,
                    SCENES
                )
                .unwrap_err(),
            SceneSwitchError::Settings
        );
        // The matching snapshot is accepted.
        let cadence = snapshot(SceneId::Loom);
        assert!(timeline
            .retarget(
                1,
                SceneId::Loom.index() as u32,
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
