//! Pure hit testing and boundary clamping for the editable scene-plan lane.
//!
//! Scene segments are a contiguous partition of the track. A body selects one
//! segment; an internal boundary belongs to the segment on its right and moves
//! both neighbours. Transition editing can later attach to that same boundary
//! identity without changing segment coverage.

use crate::project::scene_switch::{SceneSwitchTimeline, MIN_CUE_SECONDS};

use super::timed_lane;
use super::timeline_view::TimelineView;

/// Pixels on either side of an internal boundary that claim its drag.
pub const SCENE_BOUNDARY_GRAB_PIXELS: f64 = 5.0;

/// Which independently selectable part of the lane was hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneLaneZone {
    Segment,
    /// The start of this cue, hence the shared boundary with its predecessor.
    Boundary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneLaneHit {
    pub id: u64,
    pub zone: SceneLaneZone,
}

/// Hits visible internal boundaries before segment bodies. That priority is
/// load-bearing: the same point belongs to both adjacent blocks.
#[must_use]
pub fn hit_test(
    plan: &SceneSwitchTimeline,
    view: &TimelineView,
    lane_x: f64,
    lane_width: f64,
    pointer_x: f64,
) -> Option<SceneLaneHit> {
    if !pointer_x.is_finite()
        || !lane_x.is_finite()
        || !lane_width.is_finite()
        || lane_width <= 0.0
        || pointer_x < lane_x
        || pointer_x > lane_x + lane_width
    {
        return None;
    }

    for cue in plan.cues().iter().skip(1) {
        let boundary_x = view.x_at(cue.start_seconds, lane_x, lane_width);
        if boundary_x >= lane_x
            && boundary_x <= lane_x + lane_width
            && (pointer_x - boundary_x).abs() <= SCENE_BOUNDARY_GRAB_PIXELS
        {
            return Some(SceneLaneHit {
                id: cue.id,
                zone: SceneLaneZone::Boundary,
            });
        }
    }

    plan.cues().iter().rev().find_map(|cue| {
        let block = timed_lane::block_geometry(
            view,
            lane_x,
            lane_width,
            cue.start_seconds,
            cue.end_seconds,
        )?;
        block.contains_x(pointer_x).then_some(SceneLaneHit {
            id: cue.id,
            zone: SceneLaneZone::Segment,
        })
    })
}

/// Clamps a proposed internal boundary to the exact interval accepted by
/// `SceneSwitchTimeline::retime`, so preview and commit cannot disagree.
#[must_use]
pub fn clamp_boundary(
    plan: &SceneSwitchTimeline,
    right_cue_id: u64,
    proposed_seconds: f64,
) -> Option<f64> {
    if !proposed_seconds.is_finite() {
        return None;
    }
    let index = plan.cues().iter().position(|cue| cue.id == right_cue_id)?;
    if index == 0 {
        return None;
    }
    let left = plan.cues()[index - 1].start_seconds + MIN_CUE_SECONDS;
    let right = plan.cues()[index].end_seconds - MIN_CUE_SECONDS;
    if right < left {
        return None;
    }
    Some(proposed_seconds.clamp(left, right))
}

/// What a segment carries beyond its scene, in words, for the lane's name
/// field and the report (MV9-k): what its threshold doorway shows, how it
/// arrives, whether it glides and whether it is graded. `None` for a segment
/// that is just its scene.
///
/// `base` is the track's per-scene table, which a cue's snapshot is applied
/// over; the threshold's `beyond` is read from the result.
#[must_use]
pub fn segment_summary(
    cue: &crate::project::scene_switch::SceneSwitchCue,
    base: &crate::scene::SceneSettings,
) -> Option<String> {
    use crate::scene::settings::index::threshold;
    use crate::scene::SceneId;
    let scene = SceneId::from_index(cue.scene_index as usize)?;
    let mut parts = Vec::new();
    if scene == SceneId::Threshold {
        let mut table = *base;
        if cue.settings.captured {
            table.apply_snapshot(scene, &cue.settings);
        }
        let beyond = crate::scenes::threshold::beyond_scene(table.get(scene, threshold::BEYOND));
        parts.push(format!(
            "door onto {}",
            beyond.map_or("its fog", SceneId::display_name)
        ));
    }
    if !cue.transition.is_cut() {
        parts.push(format!(
            "arrives by {} {:.1} s",
            cue.transition.kind, cue.transition.seconds
        ));
    }
    if cue.glide > 0.0 {
        parts.push(format!("glides {:.1} s", cue.glide));
    }
    if let Some(grade) = cue.grade.filter(|grade| !grade.is_identity()) {
        if grade.map > 0.0 {
            parts.push(format!("graded {:.0}%", grade.map * 100.0));
        } else {
            parts.push("hue-shifted".to_owned());
        }
    }
    if let Some(look) = cue.pixel {
        parts.push(if look.is_on() {
            format!("pixel {} {}", look.palette_label(), look.pixels)
        } else {
            "pixel off".to_owned()
        });
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_summary_names_what_the_lane_cannot_show_by_colour() {
        use crate::project::look::{Grade, Palette};
        use crate::project::scene_switch::SceneSwitchCue;
        use crate::project::transition::{Transition, TransitionKind};
        use crate::scene::settings::index::threshold;
        use crate::scene::{SceneId, SceneSettings};
        let base = SceneSettings::new();
        let plain = SceneSwitchCue {
            id: 1,
            start_seconds: 0.0,
            end_seconds: 10.0,
            scene_index: SceneId::Loom.index() as u32,
            ..SceneSwitchCue::default()
        };
        assert_eq!(segment_summary(&plain, &base), None);
        let dressed = SceneSwitchCue {
            transition: Transition::new(TransitionKind::Portal, 0.8),
            glide: 2.6,
            grade: Some(Grade::from_palette(Palette::NightDoor, 0.82)),
            ..plain
        };
        assert_eq!(
            segment_summary(&dressed, &base).as_deref(),
            Some("arrives by portal 0.8 s · glides 2.6 s · graded 82%")
        );
        use crate::project::pixel::Pixel;
        let pixelled = SceneSwitchCue {
            pixel: Some(Pixel::on()),
            ..plain
        };
        assert_eq!(
            segment_summary(&pixelled, &base).as_deref(),
            Some("pixel handheld 180")
        );
        let unpixelled = SceneSwitchCue {
            pixel: Some(Pixel::OFF),
            ..plain
        };
        assert_eq!(
            segment_summary(&unpixelled, &base).as_deref(),
            Some("pixel off")
        );
        let mut tuned = base;
        tuned.set(SceneId::Threshold, threshold::BEYOND, 7.0);
        let door = SceneSwitchCue {
            scene_index: SceneId::Threshold.index() as u32,
            settings: tuned.capture(SceneId::Threshold).unwrap(),
            ..plain
        };
        assert_eq!(
            segment_summary(&door, &base).as_deref(),
            Some("door onto Constellation")
        );
    }
    use crate::project::scene_switch::SceneSwitchCue;
    use crate::scene::{SceneId, SceneSettings};

    fn plan() -> SceneSwitchTimeline {
        let settings = SceneSettings::new();
        let snapshot = settings.capture(SceneId::Spectrum).unwrap();
        let cues = [
            SceneSwitchCue {
                id: 10,
                start_seconds: 0.0,
                end_seconds: 4.0,
                scene_index: 0,
                strength: 1.0,
                settings: snapshot,
                transition: crate::project::transition::Transition::CUT,
                grade: None,
                glide: 0.0,
                pixel: None,
            },
            SceneSwitchCue {
                id: 20,
                start_seconds: 4.0,
                end_seconds: 10.0,
                scene_index: 1,
                strength: 1.0,
                settings: settings.capture(SceneId::PulseField).unwrap(),
                transition: crate::project::transition::Transition::CUT,
                grade: None,
                glide: 0.0,
                pixel: None,
            },
        ];
        let mut plan = SceneSwitchTimeline::new();
        plan.replace(&cues, 10.0, SceneId::ALL.len() as u32)
            .unwrap();
        plan
    }

    #[test]
    fn a_shared_boundary_wins_over_both_segment_bodies() {
        let plan = plan();
        let view = TimelineView::new(10.0);
        assert_eq!(
            hit_test(&plan, &view, 0.0, 100.0, 40.0),
            Some(SceneLaneHit {
                id: 20,
                zone: SceneLaneZone::Boundary,
            })
        );
        assert_eq!(
            hit_test(&plan, &view, 0.0, 100.0, 20.0),
            Some(SceneLaneHit {
                id: 10,
                zone: SceneLaneZone::Segment,
            })
        );
    }

    #[test]
    fn a_scrolled_off_boundary_is_not_faked_at_the_lane_edge() {
        let plan = plan();
        let mut view = TimelineView::new(10.0);
        view.start_seconds = 5.0;
        view.span_seconds = 5.0;
        assert_eq!(
            hit_test(&plan, &view, 100.0, 100.0, 100.0),
            Some(SceneLaneHit {
                id: 20,
                zone: SceneLaneZone::Segment,
            })
        );
    }

    #[test]
    fn a_boundary_exactly_on_the_view_edge_remains_grabbable() {
        let plan = plan();
        let mut view = TimelineView::new(10.0);
        view.start_seconds = 4.0;
        view.span_seconds = 6.0;
        assert_eq!(
            hit_test(&plan, &view, 100.0, 120.0, 100.0),
            Some(SceneLaneHit {
                id: 20,
                zone: SceneLaneZone::Boundary,
            })
        );
        assert_eq!(clamp_boundary(&plan, 10, 2.0), None);
    }

    #[test]
    fn boundary_preview_is_exactly_what_retime_accepts() {
        let plan = plan();
        let clamped = clamp_boundary(&plan, 20, -100.0).unwrap();
        assert_eq!(clamped, MIN_CUE_SECONDS);
        let mut committed = plan.clone();
        committed
            .retime(1, clamped, 10.0, SceneId::ALL.len() as u32)
            .unwrap();
        assert_eq!(committed.cues()[1].start_seconds, clamped);
        assert_eq!(committed.cues()[0].end_seconds, clamped);
    }
}
