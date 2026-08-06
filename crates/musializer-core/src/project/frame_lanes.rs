//! The project-owned lanes attached to one [`SceneFrame`](crate::scene::SceneFrame).
//!
//! This is the Rust equivalent of `make_scene_frame` in the frozen application's
//! `plug.c:1115-1185`. Preview and export both build this value, then ask it for
//! the actual frame. Keeping the owned merge beside the borrowed frame view makes
//! it impossible for a scene to retain an event slice after the merge is dropped.

use crate::project::event_timeline::EventTimeline;
use crate::project::lyrics::LyricsDocument;
use crate::project::semantic_lane;
use crate::scene::events::{EventTimelineError, SceneEventMerge};
use crate::scene::{LyricCue, SceneAudioFrame, SceneFrame, SceneSettings, SemanticFrame};

/// Clock and identity fields that do not belong to a project lane.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneFrameTiming {
    pub time_seconds: f64,
    pub duration_seconds: f64,
    pub delta_seconds: f32,
    pub frame_index: u64,
}

/// The sampled semantic/lyric lanes and the owned canonical event merge.
///
/// A fresh value is built for every frame. That is intentional: if a merge is
/// invalid, this value owns an empty merge and therefore cannot accidentally
/// expose events cached for the previous track (`plug.c:1102-1110`).
#[derive(Clone, Debug, Default)]
pub struct ProjectFrameLanes {
    semantic: SemanticFrame,
    lyric: Option<LyricCue>,
    events: SceneEventMerge,
    merge_error: Option<EventTimelineError>,
}

impl ProjectFrameLanes {
    /// The no-track frame view: neutral semantics, no lyric, and no events.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Samples and merges the three project lanes at `time_seconds`.
    ///
    /// Semantic sampling and active-lyric selection use their canonical project
    /// implementations. Manual and semantic events remain separate inputs until
    /// [`SceneEventMerge`] qualifies semantic ids and sorts the combined view.
    #[must_use]
    pub fn build(
        time_seconds: f64,
        lyrics: &LyricsDocument,
        semantic_events: &EventTimeline,
        manual_events: &EventTimeline,
    ) -> Self {
        Self::build_from_slices(
            time_seconds,
            lyrics,
            semantic_events.events(),
            manual_events.events(),
        )
    }

    fn build_from_slices(
        time_seconds: f64,
        lyrics: &LyricsDocument,
        semantic_events: &[crate::scene::events::EventRecord],
        manual_events: &[crate::scene::events::EventRecord],
    ) -> Self {
        let sampled = semantic_lane::sample(
            crate::scene::events::EventTimelineView {
                events: semantic_events,
            },
            time_seconds,
        )
        .unwrap_or_default();
        let semantic = SemanticFrame {
            available: sampled.available,
            source_id: sampled.source_id,
            energy: sampled.energy,
            tension: sampled.tension,
            valence: sampled.valence,
            confidence: sampled.confidence,
        };
        let lyric = lyrics.at_time(time_seconds).map(|cue| LyricCue {
            id: cue.id,
            start_seconds: cue.start_seconds,
            end_seconds: cue.end_seconds,
            text: cue.text.clone(),
        });

        let mut events = SceneEventMerge::new();
        let merge_error = events.build(manual_events, semantic_events).err();
        if merge_error.is_some() {
            // `SceneEventMerge::build` validates before clearing its destination.
            // This is a fresh destination today, but clearing explicitly pins the
            // application boundary's stronger promise if the implementation is
            // ever changed to reuse allocations.
            events.clear();
        }
        Self {
            semantic,
            lyric,
            events,
            merge_error,
        }
    }

    /// Why the merged event view was replaced by an empty one, if anything.
    #[must_use]
    pub fn merge_error(&self) -> Option<EventTimelineError> {
        self.merge_error
    }

    /// A compact state report used by headless preview/export evidence.
    #[must_use]
    pub fn status(&self) -> FrameLaneStatus {
        FrameLaneStatus {
            lyric_id: self.lyric.as_ref().map(|cue| cue.id),
            semantic_available: self.semantic.available,
            semantic_source_id: self.semantic.source_id,
            merged_event_count: self.events.view().len(),
        }
    }

    /// Attaches these project lanes to the audio, clock and effective settings.
    #[must_use]
    pub fn scene_frame<'frame>(
        &'frame self,
        timing: SceneFrameTiming,
        audio: SceneAudioFrame<'frame>,
        settings: &'frame SceneSettings,
    ) -> SceneFrame<'frame> {
        SceneFrame {
            time_seconds: timing.time_seconds,
            duration_seconds: timing.duration_seconds,
            delta_seconds: timing.delta_seconds,
            frame_index: timing.frame_index,
            audio,
            semantic: self.semantic,
            lyric: self.lyric.as_ref(),
            events: self.events.view(),
            settings,
        }
    }
}

/// Observable project-lane state at one scene time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameLaneStatus {
    pub lyric_id: Option<u64>,
    pub semantic_available: bool,
    pub semantic_source_id: u64,
    pub merged_event_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::lyrics::LyricCue as ProjectLyricCue;
    use crate::scene::events::{EventRecord, EventType, SEMANTIC_ID_LANE_BIT};

    fn event(time: f64, id: u64, kind: EventType, values: [f32; 4], count: u8) -> EventRecord {
        EventRecord {
            timestamp_seconds: time,
            id,
            event_type: kind as u32,
            value_count: count,
            values,
        }
    }

    fn lyrics() -> LyricsDocument {
        let mut lyrics = LyricsDocument::new(12.0).unwrap();
        lyrics
            .insert(ProjectLyricCue {
                id: 9,
                start_seconds: 2.0,
                end_seconds: 4.0,
                text: "Καλημέρα, мир".into(),
                origin: Default::default(),
            })
            .unwrap();
        lyrics
    }

    #[test]
    fn boundary_sampling_and_merge_are_one_frame_view() {
        let lyrics = lyrics();
        let mut semantic = EventTimeline::new();
        semantic
            .record(event(2.0, 5, EventType::Semantic, [0.4, 0.6, -0.2, 0.9], 4))
            .unwrap();
        let mut manual = EventTimeline::new();
        manual
            .record(event(2.0, 7, EventType::Cue, [1.0, 0.0, 0.0, 0.0], 1))
            .unwrap();

        let before = ProjectFrameLanes::build(1.999, &lyrics, &semantic, &manual);
        assert_eq!(before.status().lyric_id, None);
        assert!(!before.status().semantic_available);
        assert_eq!(before.status().merged_event_count, 2);

        let at = ProjectFrameLanes::build(2.0, &lyrics, &semantic, &manual);
        assert_eq!(at.status().lyric_id, Some(9));
        assert_eq!(at.status().semantic_source_id, 5);
        let ids: Vec<_> = at
            .events
            .view()
            .events
            .iter()
            .map(|event| event.id)
            .collect();
        assert_eq!(ids, [5 ^ SEMANTIC_ID_LANE_BIT, 7]);

        let end = ProjectFrameLanes::build(4.0, &lyrics, &semantic, &manual);
        assert_eq!(end.status().lyric_id, None, "lyric ends are exclusive");
        assert!(
            end.status().semantic_available,
            "semantic values hold forward"
        );
    }

    #[test]
    fn an_invalid_merge_is_reported_and_cannot_retain_old_events() {
        let lyrics = lyrics();
        let valid = [event(1.0, 1, EventType::Cue, [1.0, 0.0, 0.0, 0.0], 1)];
        let invalid = [
            event(2.0, 2, EventType::Cue, [1.0, 0.0, 0.0, 0.0], 1),
            event(1.0, 3, EventType::Cue, [1.0, 0.0, 0.0, 0.0], 1),
        ];
        let first = ProjectFrameLanes::build_from_slices(2.5, &lyrics, &[], &valid);
        assert_eq!(first.status().merged_event_count, 1);

        let failed = ProjectFrameLanes::build_from_slices(2.5, &lyrics, &[], &invalid);
        assert_eq!(failed.merge_error(), Some(EventTimelineError::Order));
        assert_eq!(failed.status().merged_event_count, 0);
    }

    #[test]
    fn no_track_is_the_documented_neutral_view() {
        let empty = ProjectFrameLanes::empty();
        assert_eq!(empty.status(), FrameLaneStatus::default());
        assert_eq!(empty.merge_error(), None);
    }
}
