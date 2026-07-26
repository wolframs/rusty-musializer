//! The bounded manual event timeline and its replay cursors.
//!
//! **Owner: Agent B.** Port of `../musializer/src/event_timeline.c/.h`.
//!
//! The frame-facing half of this C module already landed as
//! [`crate::scene::events`] (the record, the view, the merge, the error enum), so
//! this module adds only what the *model* needs and reuses those types rather
//! than declaring a second `EventRecord`.
//!
//! One divergence to know about: [`crate::scene::events::EventRecord::is_well_formed`]
//! is a **weaker** check than C's `event_record_is_valid`
//! (`event_timeline.c:32-44`) — it omits `id != 0`, the event-type range, and
//! `value_count != 0`. [`record_is_valid`] here is the complete C rule, and it is
//! what the timeline and the `.musi` validator use. Do not swap one for the other.

use crate::scene::events::{
    EventRecord, EventTimelineError, EventTimelineView, EventType, TIMELINE_CAPACITY,
    VALUE_CAPACITY,
};

/// Canonical ordering key: `(timestamp, type, id)` (`event_timeline.c:6-15`).
///
/// Recording inserts by this key so replay is deterministic regardless of the
/// order in which producers submit events.
fn compare(left: &EventRecord, right: &EventRecord) -> core::cmp::Ordering {
    left.timestamp_seconds
        .partial_cmp(&right.timestamp_seconds)
        .unwrap_or(core::cmp::Ordering::Equal)
        .then(left.event_type.cmp(&right.event_type))
        .then(left.id.cmp(&right.id))
}

/// The complete C validity rule for one record (`event_timeline.c:32-44`).
///
/// `id == 0` is reserved (it is the "allocate me one" sentinel elsewhere in the
/// model), and the event type must be one of the four named kinds — an unknown
/// type is rejected here even though [`EventRecord`] stores the raw `u32` so a
/// future kind can survive a round trip through memory.
#[must_use]
pub fn record_is_valid(event: &EventRecord) -> bool {
    if !event.timestamp_seconds.is_finite()
        || event.timestamp_seconds < 0.0
        || event.id == 0
        || EventType::from_raw(event.event_type).is_none()
        || event.value_count == 0
        || event.value_count as usize > VALUE_CAPACITY
    {
        return false;
    }
    event.values().iter().all(|value| value.is_finite())
}

/// A bounded, canonically ordered lane of events (`Event_Timeline`,
/// `event_timeline.h:30-34`).
///
/// The C capacity is fixed at 1024 so "project loading and live recording can
/// never grow the realtime state without a caller-visible overflow result". The
/// Rust storage is a `Vec` for convenience, but every mutation still enforces
/// [`TIMELINE_CAPACITY`], because that bound is the invariant, not the array.
#[derive(Clone, Debug)]
pub struct EventTimeline {
    events: Vec<EventRecord>,
    revision: u64,
}

impl Default for EventTimeline {
    /// `event_timeline_init` (`event_timeline.c:17-22`): empty, revision 1.
    fn default() -> Self {
        Self {
            events: Vec::new(),
            revision: 1,
        }
    }
}

impl EventTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn events(&self) -> &[EventRecord] {
        &self.events
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Revision counter. Skips zero on wrap so zero can never mean "current"
    /// (`event_timeline.c:29`).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if self.revision == 0 {
            self.revision = 1;
        }
    }

    /// Builds a lane holding events exactly as a file listed them, for the `.musi`
    /// codec.
    ///
    /// Deliberately **not** [`Self::record`], which inserts in canonical order and
    /// would therefore *fix* an unsorted file instead of rejecting it. A document
    /// that round-trips from unsorted to sorted has been silently rewritten, and
    /// the caller's [`Self::validate`] is what refuses it instead.
    pub(crate) fn from_file_order(events: Vec<EventRecord>) -> Self {
        Self {
            events,
            revision: 1,
        }
    }

    /// `event_timeline_clear` (`event_timeline.c:24-30`). Advances the revision
    /// even though nothing is left, which is what invalidates live cursors.
    pub fn clear(&mut self) {
        self.events.clear();
        self.bump_revision();
    }

    /// `event_timeline_validate` (`event_timeline.c:46-65`).
    pub fn validate(&self) -> Result<(), EventTimelineError> {
        if self.events.len() > TIMELINE_CAPACITY {
            return Err(EventTimelineError::Overflow);
        }
        for (index, event) in self.events.iter().enumerate() {
            if !record_is_valid(event) {
                return Err(EventTimelineError::Malformed);
            }
            if index > 0 && compare(&self.events[index - 1], event) != core::cmp::Ordering::Less {
                // C reports a duplicate id in preference to an ordering fault
                // when both apply, because that is the more actionable message.
                if self.events[index - 1].id == event.id {
                    return Err(EventTimelineError::DuplicateId);
                }
                return Err(EventTimelineError::Order);
            }
            if self.events[..index]
                .iter()
                .any(|previous| previous.id == event.id)
            {
                return Err(EventTimelineError::DuplicateId);
            }
        }
        Ok(())
    }

    /// `event_timeline_record` (`event_timeline.c:67-93`): insert at the canonical
    /// position, rejecting duplicate ids and overflow.
    pub fn record(&mut self, event: EventRecord) -> Result<(), EventTimelineError> {
        if !record_is_valid(&event) {
            return Err(EventTimelineError::Malformed);
        }
        if self.events.iter().any(|existing| existing.id == event.id) {
            return Err(EventTimelineError::DuplicateId);
        }
        if self.events.len() >= TIMELINE_CAPACITY {
            return Err(EventTimelineError::Overflow);
        }
        let at = self
            .events
            .partition_point(|existing| compare(existing, &event) == core::cmp::Ordering::Less);
        self.events.insert(at, event);
        self.bump_revision();
        Ok(())
    }

    /// `event_timeline_replace` (`event_timeline.c:95-106`).
    ///
    /// Validates the source *before* replacing anything and advances the
    /// destination's revision even when both lanes hold the same number of
    /// events, so a cursor cannot survive a swap that changed the contents.
    pub fn replace(&mut self, source: &EventTimeline) -> Result<(), EventTimelineError> {
        source.validate()?;
        let next = self.revision.wrapping_add(1);
        let revision = if next == 0 { 1 } else { next };
        self.events.clear();
        self.events.extend_from_slice(&source.events);
        self.revision = revision;
        Ok(())
    }

    /// `event_timeline_view` (`event_timeline.c:108-114`).
    #[must_use]
    pub fn view(&self) -> EventTimelineView<'_> {
        if self.events.len() > TIMELINE_CAPACITY {
            return EventTimelineView::EMPTY;
        }
        EventTimelineView {
            events: &self.events,
        }
    }

    /// `event_timeline_cursor_begin` (`event_timeline.c:116-128`).
    pub fn cursor(&self) -> Result<EventTimelineCursor<'_>, EventTimelineError> {
        self.validate()?;
        Ok(EventTimelineCursor {
            timeline: self,
            index: 0,
            position_seconds: 0.0,
        })
    }
}

/// A replay cursor over one timeline (`Event_Timeline_Cursor`,
/// `event_timeline.h:54-59`).
///
/// C carries the timeline's revision and returns `ERROR_STALE_CURSOR` when the
/// timeline changed mid-replay (`event_timeline.c:167-169`). That case is
/// **unrepresentable here**: the cursor holds a shared borrow of the timeline, so
/// no `&mut` path exists while it is alive. The revision field is therefore
/// omitted rather than carried as dead weight, and the C stale-cursor test has no
/// Rust counterpart on purpose.
#[derive(Clone, Debug)]
pub struct EventTimelineCursor<'a> {
    timeline: &'a EventTimeline,
    index: usize,
    position_seconds: f64,
}

impl<'a> EventTimelineCursor<'a> {
    #[must_use]
    pub fn position_seconds(&self) -> f64 {
        self.position_seconds
    }

    /// `event_timeline_cursor_seek` (`event_timeline.c:130-154`).
    ///
    /// Seeks to the first event at or after `timestamp_seconds`.
    pub fn seek(&mut self, timestamp_seconds: f64) -> Result<(), EventTimelineError> {
        if !timestamp_seconds.is_finite() || timestamp_seconds < 0.0 {
            return Err(EventTimelineError::Malformed);
        }
        self.timeline.validate()?;
        self.index = self
            .timeline
            .events
            .partition_point(|event| event.timestamp_seconds < timestamp_seconds);
        self.position_seconds = timestamp_seconds;
        Ok(())
    }

    /// `event_timeline_cursor_next_until` (`event_timeline.c:156-178`).
    ///
    /// `Ok(None)` is C's `EVENT_TIMELINE_DONE`: no further event lies at or before
    /// `inclusive_end_seconds`, and the cursor's position advances to that end so
    /// the next call resumes from there.
    pub fn next_until(
        &mut self,
        inclusive_end_seconds: f64,
    ) -> Result<Option<&'a EventRecord>, EventTimelineError> {
        if !inclusive_end_seconds.is_finite() || inclusive_end_seconds < self.position_seconds {
            return Err(EventTimelineError::Malformed);
        }
        match self.timeline.events.get(self.index) {
            Some(event) if event.timestamp_seconds <= inclusive_end_seconds => {
                self.index += 1;
                self.position_seconds = event.timestamp_seconds;
                Ok(Some(event))
            }
            _ => {
                self.position_seconds = inclusive_end_seconds;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(timestamp: f64, id: u64, kind: EventType) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: kind as u32,
            value_count: 1,
            values: [0.5, 0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn a_fresh_timeline_starts_at_revision_one() {
        let timeline = EventTimeline::new();
        assert_eq!(timeline.revision(), 1);
        assert!(timeline.is_empty());
        assert!(timeline.validate().is_ok());
    }

    #[test]
    fn malformed_records_are_rejected_field_by_field() {
        let mut timeline = EventTimeline::new();
        let base = event(1.0, 1, EventType::Cue);
        let cases = [
            EventRecord { id: 0, ..base },
            EventRecord {
                timestamp_seconds: -0.001,
                ..base
            },
            EventRecord {
                timestamp_seconds: f64::NAN,
                ..base
            },
            EventRecord {
                timestamp_seconds: f64::INFINITY,
                ..base
            },
            EventRecord {
                event_type: 0,
                ..base
            },
            EventRecord {
                event_type: 5,
                ..base
            },
            EventRecord {
                value_count: 0,
                ..base
            },
            EventRecord {
                value_count: 5,
                ..base
            },
            EventRecord {
                values: [f32::NAN, 0.0, 0.0, 0.0],
                ..base
            },
        ];
        for candidate in cases {
            assert!(!record_is_valid(&candidate), "{candidate:?}");
            assert_eq!(
                timeline.record(candidate),
                Err(EventTimelineError::Malformed)
            );
        }
        assert!(timeline.is_empty());
    }

    #[test]
    fn recording_orders_by_timestamp_then_type_then_id() {
        let mut timeline = EventTimeline::new();
        timeline.record(event(2.0, 10, EventType::Cue)).unwrap();
        timeline.record(event(1.0, 11, EventType::Lyric)).unwrap();
        timeline.record(event(2.0, 12, EventType::Lyric)).unwrap();
        timeline.record(event(2.0, 9, EventType::Cue)).unwrap();
        let order: Vec<u64> = timeline.events().iter().map(|event| event.id).collect();
        assert_eq!(order, vec![11, 12, 9, 10]);
        assert!(timeline.validate().is_ok());
    }

    #[test]
    fn duplicate_ids_are_refused_and_change_nothing() {
        let mut timeline = EventTimeline::new();
        timeline.record(event(1.0, 7, EventType::Cue)).unwrap();
        let revision = timeline.revision();
        assert_eq!(
            timeline.record(event(4.0, 7, EventType::Custom)),
            Err(EventTimelineError::DuplicateId)
        );
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline.revision(), revision);
    }

    #[test]
    fn overflow_is_reported_rather_than_growing_the_lane() {
        let mut timeline = EventTimeline::new();
        for index in 0..TIMELINE_CAPACITY {
            timeline
                .record(event(
                    index as f64 * 0.001,
                    index as u64 + 1,
                    EventType::Cue,
                ))
                .unwrap();
        }
        assert_eq!(timeline.len(), TIMELINE_CAPACITY);
        assert_eq!(
            timeline.record(event(9999.0, 999_999, EventType::Cue)),
            Err(EventTimelineError::Overflow)
        );
    }

    #[test]
    fn replace_validates_the_source_and_always_advances_the_revision() {
        let mut destination = EventTimeline::new();
        destination.record(event(1.0, 1, EventType::Cue)).unwrap();
        let before = destination.revision();

        let mut source = EventTimeline::new();
        source.record(event(3.0, 2, EventType::Lyric)).unwrap();
        destination.replace(&source).unwrap();
        assert_eq!(destination.revision(), before + 1);
        assert_eq!(destination.len(), 1);
        assert_eq!(destination.events()[0].id, 2);

        // Same event count, still a new revision.
        let mut other = EventTimeline::new();
        other.record(event(4.0, 3, EventType::Lyric)).unwrap();
        let before = destination.revision();
        destination.replace(&other).unwrap();
        assert_eq!(destination.revision(), before + 1);
    }

    #[test]
    fn cursor_replays_each_window_once_and_reports_done() {
        let mut timeline = EventTimeline::new();
        timeline.record(event(0.5, 1, EventType::Cue)).unwrap();
        timeline.record(event(1.5, 2, EventType::Cue)).unwrap();
        timeline.record(event(2.5, 3, EventType::Cue)).unwrap();

        let mut cursor = timeline.cursor().unwrap();
        assert_eq!(cursor.next_until(1.0).unwrap().map(|e| e.id), Some(1));
        assert_eq!(cursor.next_until(1.0).unwrap().map(|e| e.id), None);
        assert_eq!(cursor.position_seconds(), 1.0);
        assert_eq!(cursor.next_until(2.0).unwrap().map(|e| e.id), Some(2));
        assert_eq!(cursor.next_until(2.0).unwrap().map(|e| e.id), None);

        // A window ending before the cursor's position is malformed, not empty.
        assert_eq!(
            cursor.next_until(0.5).unwrap_err(),
            EventTimelineError::Malformed
        );
    }

    #[test]
    fn seek_positions_at_the_first_event_at_or_after_the_time() {
        let mut timeline = EventTimeline::new();
        timeline.record(event(1.0, 1, EventType::Cue)).unwrap();
        timeline.record(event(2.0, 2, EventType::Cue)).unwrap();

        let mut cursor = timeline.cursor().unwrap();
        cursor.seek(2.0).unwrap();
        assert_eq!(cursor.next_until(9.0).unwrap().map(|e| e.id), Some(2));

        cursor.seek(0.0).unwrap();
        assert_eq!(cursor.next_until(9.0).unwrap().map(|e| e.id), Some(1));
        assert_eq!(cursor.seek(-1.0), Err(EventTimelineError::Malformed));
        assert_eq!(cursor.seek(f64::NAN), Err(EventTimelineError::Malformed));
    }
}
