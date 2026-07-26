//! The frame-facing event view and its merge.
//!
//! **Shared contract.** Consumed by Agents C and D. Port of
//! `../musializer/src/event_timeline.h` and `scene_event_merge.h`/`.c`.
//!
//! Two things here are load-bearing:
//!
//! - The manual and semantic lanes are *independently* bounded, so the merged
//!   frame-facing view has to be able to carry both without dropping the second
//!   (`scene_event_merge.h:6-8`).
//! - Semantic ids are qualified into a separate display namespace, so equal ids
//!   in two independently authored lanes stay distinct to a scene renderer
//!   (`scene_event_merge.h:15-17`). Merging without that would let a semantic
//!   event masquerade as a manual one, collapsing two evidence lanes into one.

/// Fixed capacity of one timeline (`event_timeline.h:10`).
///
/// Deliberately fixed in C: project loading and live recording can never grow
/// realtime state without a caller-visible overflow result.
pub const TIMELINE_CAPACITY: usize = 1024;
/// Values one event carries (`event_timeline.h:11`).
pub const VALUE_CAPACITY: usize = 4;
/// Merged-view capacity: both lanes at full size (`scene_event_merge.h:8`).
pub const MERGE_CAPACITY: usize = TIMELINE_CAPACITY * 2;

/// Event kinds (`event_timeline.h:13-19`).
///
/// The discriminants are persisted, so they are a compatibility surface. Note
/// they start at 1, not 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum EventType {
    Lyric = 1,
    Semantic = 2,
    Cue = 3,
    Custom = 4,
}

impl EventType {
    #[must_use]
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(EventType::Lyric),
            2 => Some(EventType::Semantic),
            3 => Some(EventType::Cue),
            4 => Some(EventType::Custom),
            _ => None,
        }
    }
}

/// One recorded event (`event_timeline.h:21-28`).
///
/// C's `reserved[3]` padding is omitted: it exists to make the C struct's layout
/// explicit, and Rust has no equivalent need here since nothing serializes this
/// struct byte-wise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventRecord {
    pub timestamp_seconds: f64,
    pub id: u64,
    /// Kept as a raw `u32` rather than an [`EventType`] because a project may
    /// carry a type this build does not know, and dropping it silently would be
    /// worse than passing it through.
    pub event_type: u32,
    pub value_count: u8,
    pub values: [f32; VALUE_CAPACITY],
}

impl Default for EventRecord {
    fn default() -> Self {
        Self {
            timestamp_seconds: 0.0,
            id: 0,
            event_type: 0,
            value_count: 0,
            values: [0.0; VALUE_CAPACITY],
        }
    }
}

impl EventRecord {
    /// The values this event actually carries.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values[..(self.value_count as usize).min(VALUE_CAPACITY)]
    }

    #[must_use]
    pub fn kind(&self) -> Option<EventType> {
        EventType::from_raw(self.event_type)
    }

    /// Bounds and finiteness, the checks the C timeline applies on record and
    /// load.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.timestamp_seconds.is_finite()
            && self.timestamp_seconds >= 0.0
            && (self.value_count as usize) <= VALUE_CAPACITY
            && self.values().iter().all(|value| value.is_finite())
    }
}

/// A non-owning, immutable, canonically ordered event view for one frame
/// (`event_timeline.h:36-41`).
///
/// The producer must keep the backing storage alive for the frame's duration —
/// which in Rust the lifetime enforces rather than merely documenting.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventTimelineView<'a> {
    pub events: &'a [EventRecord],
}

impl EventTimelineView<'_> {
    pub const EMPTY: EventTimelineView<'static> = EventTimelineView { events: &[] };

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events whose timestamp falls in `[start, end]`, inclusive at both ends as
    /// the C cursor's `next_until` is.
    pub fn between(&self, start: f64, end: f64) -> impl Iterator<Item = &EventRecord> + '_ {
        self.events
            .iter()
            .filter(move |event| event.timestamp_seconds >= start && event.timestamp_seconds <= end)
    }
}

/// Why a timeline operation failed (`event_timeline.h:43-52`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EventTimelineError {
    #[error("event record is malformed")]
    Malformed,
    #[error("duplicate event id")]
    DuplicateId,
    #[error("event timeline is full")]
    Overflow,
    #[error("events are out of order")]
    Order,
}

/// Offset applied to semantic ids in the merged view, keeping them in a display
/// namespace of their own (`scene_event_merge.c`).
///
/// The value only has to be larger than any id a manual lane can hold; the high
/// bit is the cheapest such choice and makes a qualified id obvious in a debugger.
pub const SEMANTIC_ID_NAMESPACE: u64 = 1u64 << 63;

/// The merged, canonically ordered view of the manual and semantic lanes.
#[derive(Clone, Debug, Default)]
pub struct SceneEventMerge {
    events: Vec<EventRecord>,
}

impl SceneEventMerge {
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Builds one deterministic, canonically ordered view of both lanes.
    ///
    /// Manual ids are preserved; semantic ids are qualified by
    /// [`SEMANTIC_ID_NAMESPACE`]. Ordering is by `(timestamp, id)` so the result
    /// is deterministic for identical inputs, which is what lets a scene's output
    /// stay reproducible.
    ///
    /// # Errors
    /// [`EventTimelineError::Overflow`] when the two lanes together exceed
    /// [`MERGE_CAPACITY`]; [`EventTimelineError::Malformed`] for a record that
    /// fails [`EventRecord::is_well_formed`].
    pub fn build(
        &mut self,
        manual: &[EventRecord],
        semantic: &[EventRecord],
    ) -> Result<(), EventTimelineError> {
        if manual.len() + semantic.len() > MERGE_CAPACITY {
            return Err(EventTimelineError::Overflow);
        }
        if !manual
            .iter()
            .chain(semantic)
            .all(EventRecord::is_well_formed)
        {
            return Err(EventTimelineError::Malformed);
        }

        self.events.clear();
        self.events.extend_from_slice(manual);
        self.events.extend(semantic.iter().map(|event| EventRecord {
            id: event.id | SEMANTIC_ID_NAMESPACE,
            ..*event
        }));
        // Stable canonical order. `total_cmp` rather than `partial_cmp` because
        // the sort must be total; malformed timestamps were already rejected.
        self.events.sort_by(|a, b| {
            a.timestamp_seconds
                .total_cmp(&b.timestamp_seconds)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(())
    }

    #[must_use]
    pub fn view(&self) -> EventTimelineView<'_> {
        EventTimelineView {
            events: &self.events,
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(timestamp: f64, id: u64, event_type: EventType) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: event_type as u32,
            ..Default::default()
        }
    }

    #[test]
    fn event_type_discriminants_start_at_one() {
        assert_eq!(EventType::Lyric as u32, 1);
        assert_eq!(EventType::Custom as u32, 4);
        assert_eq!(EventType::from_raw(0), None);
        assert_eq!(EventType::from_raw(5), None);
    }

    #[test]
    fn an_unknown_event_type_survives_round_tripping() {
        let record = EventRecord {
            event_type: 99,
            ..Default::default()
        };
        assert_eq!(record.kind(), None);
        assert_eq!(
            record.event_type, 99,
            "an unknown type is carried, not dropped"
        );
    }

    #[test]
    fn merging_qualifies_semantic_ids_so_equal_ids_stay_distinct() {
        let manual = [event(1.0, 7, EventType::Cue)];
        let semantic = [event(2.0, 7, EventType::Semantic)];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &semantic).unwrap();
        let view = merge.view();
        assert_eq!(view.len(), 2);
        assert_eq!(view.events[0].id, 7, "the manual id is preserved");
        assert_eq!(view.events[1].id, 7 | SEMANTIC_ID_NAMESPACE);
        assert_ne!(view.events[0].id, view.events[1].id);
    }

    #[test]
    fn merging_is_ordered_and_deterministic() {
        let manual = [event(3.0, 1, EventType::Cue), event(1.0, 2, EventType::Cue)];
        let semantic = [event(2.0, 3, EventType::Semantic)];
        let mut first = SceneEventMerge::new();
        first.build(&manual, &semantic).unwrap();
        let mut second = SceneEventMerge::new();
        second.build(&manual, &semantic).unwrap();
        let stamps: Vec<f64> = first
            .view()
            .events
            .iter()
            .map(|e| e.timestamp_seconds)
            .collect();
        assert_eq!(stamps, vec![1.0, 2.0, 3.0]);
        assert_eq!(first.view().events, second.view().events);
    }

    #[test]
    fn both_lanes_fit_at_full_size() {
        let manual = vec![event(1.0, 1, EventType::Cue); TIMELINE_CAPACITY];
        let semantic = vec![event(1.0, 1, EventType::Semantic); TIMELINE_CAPACITY];
        let mut merge = SceneEventMerge::new();
        merge
            .build(&manual, &semantic)
            .expect("the merged view must carry both lanes");
        assert_eq!(merge.view().len(), MERGE_CAPACITY);

        let too_many = vec![event(1.0, 1, EventType::Cue); TIMELINE_CAPACITY + 1];
        assert_eq!(
            merge.build(&too_many, &semantic).unwrap_err(),
            EventTimelineError::Overflow
        );
    }

    #[test]
    fn malformed_records_are_rejected_before_they_reach_a_scene() {
        let mut merge = SceneEventMerge::new();
        let nan = EventRecord {
            timestamp_seconds: f64::NAN,
            ..Default::default()
        };
        assert_eq!(
            merge.build(&[nan], &[]).unwrap_err(),
            EventTimelineError::Malformed
        );

        let negative = EventRecord {
            timestamp_seconds: -1.0,
            ..Default::default()
        };
        assert_eq!(
            merge.build(&[negative], &[]).unwrap_err(),
            EventTimelineError::Malformed
        );

        let bad_values = EventRecord {
            value_count: 1,
            values: [f32::INFINITY, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        assert_eq!(
            merge.build(&[bad_values], &[]).unwrap_err(),
            EventTimelineError::Malformed
        );

        let too_many_values = EventRecord {
            value_count: 9,
            ..Default::default()
        };
        assert_eq!(
            merge.build(&[too_many_values], &[]).unwrap_err(),
            EventTimelineError::Malformed
        );
    }

    #[test]
    fn between_is_inclusive_at_both_ends() {
        let manual = [
            event(1.0, 1, EventType::Cue),
            event(2.0, 2, EventType::Cue),
            event(3.0, 3, EventType::Cue),
        ];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &[]).unwrap();
        let found: Vec<u64> = merge.view().between(1.0, 2.0).map(|e| e.id).collect();
        assert_eq!(found, vec![1, 2]);
    }

    #[test]
    fn an_empty_view_is_usable() {
        let view = EventTimelineView::EMPTY;
        assert!(view.is_empty());
        assert_eq!(view.between(0.0, 100.0).count(), 0);
    }
}
