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

// ---------------------------------------------------------------------------
// The manual event row's policy (Agent L).
//
// Everything below is `plug.c:1959-1977` and `:2834-2971` with the drawing taken
// out, so the row that a headless capture photographs and the state machine that
// a test drives are the same code.
// ---------------------------------------------------------------------------

/// The single value a manually recorded marker carries (`plug.c:1972`).
///
/// **One** value, which is the whole reason the row's caption says only
/// Constellation reacts: [`crate::project::semantic_lane::sample`] requires the
/// four-value analysis payload and skips anything else, so a `+ Feel` marker is
/// invisible to every scene that reads the semantic lane. The generic event path
/// is the only reader it has.
pub const MANUAL_MARKER_VALUE: f32 = 1.0;

/// Builds the record `record_timeline_event` builds (`plug.c:1959-1977`).
///
/// `None` when the id space is exhausted, which is the C's `UINT64_MAX` guard —
/// an id of `u64::MAX` cannot be followed by another, and
/// [`crate::project::model`]'s next-id rule refuses to wrap to zero.
#[must_use]
pub fn manual_marker(
    timestamp_seconds: f64,
    next_id: u64,
    event_type: EventType,
) -> Option<EventRecord> {
    if next_id == u64::MAX {
        return None;
    }
    let mut values = [0.0f32; VALUE_CAPACITY];
    values[0] = MANUAL_MARKER_VALUE;
    Some(EventRecord {
        timestamp_seconds,
        id: next_id,
        event_type: event_type as u32,
        value_count: 1,
        values,
    })
}

/// The three faces of the `Clear manual` button (`plug.c:2852-2855`).
///
/// The order is not interchangeable: an available undo wins over an armed
/// confirmation, so the button that just cleared the lane offers to put it back
/// rather than asking to clear the now-empty lane again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearButton {
    /// Idle. A click arms the confirmation.
    Clear,
    /// Armed. A click clears, and the button is drawn in the danger style.
    Confirm,
    /// The lane was cleared and can be put back.
    Undo,
}

impl ClearButton {
    /// `clear_label` (`plug.c:2852-2855`).
    #[must_use]
    pub fn resolve(undo_available: bool, armed: bool) -> Self {
        if undo_available {
            Self::Undo
        } else if armed {
            Self::Confirm
        } else {
            Self::Clear
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear manual",
            Self::Confirm => "Confirm clear",
            Self::Undo => "Undo clear",
        }
    }

    /// Whether the button draws in the destructive style
    /// (`danger_text_button`, `plug.c:2926-2928`).
    ///
    /// Only the armed state does. Undo is a *recovery*, and colouring it as a
    /// danger would tell the user the safe way out is the dangerous one.
    #[must_use]
    pub const fn is_danger(self) -> bool {
        matches!(self, Self::Confirm)
    }

    /// What a click on this face asks for.
    #[must_use]
    pub const fn click(self) -> ManualEventAction {
        match self {
            Self::Clear => ManualEventAction::ArmClear,
            Self::Confirm => ManualEventAction::Clear,
            Self::Undo => ManualEventAction::UndoClear,
        }
    }
}

/// `plug_record_event`'s lane half (`plug.c:1055-1066`).
///
/// The allocator only ever moves **forward past the id just used**, which is not
/// [`crate::project::model`]'s recompute-from-scratch rule: importing an event
/// with a low id must not rewind an allocator that has already handed out a
/// higher one. `u64::MAX` is left alone because `id + 1` would wrap to zero.
pub fn record_into(
    lane: &mut EventTimeline,
    next_id: &mut u64,
    event: EventRecord,
) -> Result<(), EventTimelineError> {
    lane.record(event)?;
    if event.id >= *next_id && event.id != u64::MAX {
        *next_id = event.id + 1;
    }
    Ok(())
}

/// The `Clear manual` button's cross-frame state: whether it is armed, and the
/// lane a confirmed clear took away (`p->clear_events_confirmation`,
/// `p->event_undo`, `p->event_undo_available`, `plug.c:250-256`).
///
/// One type rather than three fields because the three can disagree: C's
/// `event_undo` is meaningful only while `event_undo_available` is set, and an
/// armed confirmation surviving a clear is what would let a second click clear
/// the lane the first one just saved. Here "there is something to undo" is
/// `Option::is_some` and cannot be out of step with the buffer it describes.
#[derive(Clone, Debug, Default)]
pub struct ManualClear {
    armed: bool,
    undo: Option<EventTimeline>,
}

impl ManualClear {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Which face the button shows this frame (`plug.c:2852-2855`).
    #[must_use]
    pub fn button(&self) -> ClearButton {
        ClearButton::resolve(self.undo.is_some(), self.armed)
    }

    /// First click: arm the confirmation (`plug.c:2965`).
    pub fn arm(&mut self) {
        self.armed = true;
    }

    /// Second click: move `lane` into the undo slot and restart the allocator
    /// (`plug.c:2954-2957`).
    ///
    /// The allocator goes back to 1 because an empty lane can collide with
    /// nothing, and leaving it high would make every id after a clear look like
    /// it came from a lane that no longer exists.
    pub fn clear(&mut self, lane: &mut EventTimeline, next_id: &mut u64) {
        self.undo = Some(lane.clone());
        lane.clear();
        *next_id = 1;
        self.armed = false;
    }

    /// `Undo clear` (`plug.c:2943-2951`). `false` when there is nothing to undo.
    ///
    /// A saved lane that will not validate is put **back** rather than dropped:
    /// the user's markers are still in the slot, and losing them to a bug in the
    /// validator would be the one outcome undo exists to prevent.
    pub fn undo(&mut self, lane: &mut EventTimeline, next_id: &mut u64) -> bool {
        let Some(saved) = self.undo.take() else {
            return false;
        };
        if lane.replace(&saved).is_err() {
            self.undo = Some(saved);
            return false;
        }
        *next_id = next_id_for(lane);
        self.armed = false;
        true
    }

    /// A newly recorded event retires both (`plug.c:1067-1068`).
    ///
    /// Putting the pre-clear lane back afterwards would have to reconcile with an
    /// id that did not exist when the clear happened, so the C drops the offer
    /// rather than making that the user's problem.
    pub fn forget(&mut self) {
        self.armed = false;
        self.undo = None;
    }
}

/// `timeline_next_id` (`plug.c:624-634`): the smallest allocator value that
/// cannot collide with anything already in `lane`.
///
/// `u64::MAX` is skipped rather than incremented past, because `id + 1` would
/// wrap to zero and zero is not a valid id.
#[must_use]
pub fn next_id_for(lane: &EventTimeline) -> u64 {
    let mut next = 1u64;
    for event in lane.events() {
        if event.id >= next && event.id != u64::MAX {
            next = event.id + 1;
        }
    }
    next
}

/// What the manual event row asks the application to do.
///
/// A command rather than a mutation, for the reason the whole shell is: the row
/// draws inside a `BeginDrawing` pair and owns neither the track list nor the
/// transport, and a row that can be driven in a test is a row whose confirm/undo
/// sequence is assertable.
#[derive(Clone, Debug, PartialEq)]
pub enum ManualEventAction {
    /// `+ Feel` and `+ Custom`, and the `--event` command line
    /// (`plug_record_event`, `plug.c:1055-1069`).
    Record(EventRecord),
    /// `+ Scene`: cue the active scene and its current tuning at the playhead
    /// (`record_scene_cue`, `plug.c:1979-2030`). Carries nothing because
    /// everything it needs — the scene, its settings, the playhead — belongs to
    /// the application, not to the row.
    RecordSceneCue,
    /// First click on `Clear manual`: arm the confirmation and say so
    /// (`plug.c:2965-2970`).
    ArmClear,
    /// Second click: move the lane into the undo slot (`plug.c:2954-2963`).
    Clear,
    /// Put the cleared lane back (`plug.c:2943-2952`).
    UndoClear,
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

    #[test]
    fn a_manual_marker_carries_exactly_one_value() {
        // This is the fact the row's caption is about: one value, so
        // `semantic_lane::sample` (which requires four) skips it entirely.
        let marker = manual_marker(12.5, 7, EventType::Semantic).expect("id 7 is usable");
        assert_eq!(marker.timestamp_seconds, 12.5);
        assert_eq!(marker.id, 7);
        assert_eq!(marker.event_type, EventType::Semantic as u32);
        assert_eq!(marker.value_count, 1);
        assert_eq!(marker.values(), &[MANUAL_MARKER_VALUE]);
        assert!(record_is_valid(&marker));

        let view = crate::scene::events::EventTimelineView {
            events: core::slice::from_ref(&marker),
        };
        let frame = crate::project::semantic_lane::sample(view, 20.0).expect("a usable time");
        assert!(
            !frame.available,
            "a one-value marker must not reach the semantic lane"
        );
    }

    #[test]
    fn an_exhausted_id_space_refuses_to_build_a_marker() {
        // `id + 1` would wrap to zero and zero is not a valid id (plug.c:1961).
        assert!(manual_marker(1.0, u64::MAX, EventType::Custom).is_none());
        assert!(manual_marker(1.0, u64::MAX - 1, EventType::Custom).is_some());
    }

    #[test]
    fn the_clear_button_prefers_undo_over_an_armed_confirmation() {
        assert_eq!(ClearButton::resolve(false, false), ClearButton::Clear);
        assert_eq!(ClearButton::resolve(false, true), ClearButton::Confirm);
        assert_eq!(ClearButton::resolve(true, false), ClearButton::Undo);
        // Both set: the C's ternary chain tests undo first (plug.c:2852).
        assert_eq!(ClearButton::resolve(true, true), ClearButton::Undo);

        assert_eq!(ClearButton::Clear.label(), "Clear manual");
        assert_eq!(ClearButton::Confirm.label(), "Confirm clear");
        assert_eq!(ClearButton::Undo.label(), "Undo clear");

        // Only the armed face is destructive; undo is the way back out.
        assert!(!ClearButton::Clear.is_danger());
        assert!(ClearButton::Confirm.is_danger());
        assert!(!ClearButton::Undo.is_danger());

        assert_eq!(ClearButton::Clear.click(), ManualEventAction::ArmClear);
        assert_eq!(ClearButton::Confirm.click(), ManualEventAction::Clear);
        assert_eq!(ClearButton::Undo.click(), ManualEventAction::UndoClear);
    }

    #[test]
    fn the_allocator_moves_forward_past_the_id_used_and_never_back() {
        let mut lane = EventTimeline::new();
        let mut next = 1u64;
        record_into(&mut lane, &mut next, event(1.0, 1, EventType::Custom)).unwrap();
        assert_eq!(next, 2);
        record_into(&mut lane, &mut next, event(2.0, 40, EventType::Custom)).unwrap();
        assert_eq!(next, 41);
        // A later low id does not rewind it (plug.c:1063-1065).
        record_into(&mut lane, &mut next, event(3.0, 5, EventType::Custom)).unwrap();
        assert_eq!(next, 41);
        // `id + 1` would wrap to zero, so u64::MAX leaves it alone.
        record_into(
            &mut lane,
            &mut next,
            event(4.0, u64::MAX, EventType::Custom),
        )
        .unwrap();
        assert_eq!(next, 41);
    }

    #[test]
    fn a_refused_record_leaves_the_allocator_where_it_was() {
        let mut lane = EventTimeline::new();
        let mut next = 1u64;
        record_into(&mut lane, &mut next, event(1.0, 7, EventType::Cue)).unwrap();
        assert_eq!(next, 8);
        assert_eq!(
            record_into(&mut lane, &mut next, event(2.0, 7, EventType::Cue)),
            Err(EventTimelineError::DuplicateId)
        );
        assert_eq!(next, 8);
        assert_eq!(lane.len(), 1);
    }

    #[test]
    fn next_id_for_skips_u64_max_and_starts_at_one() {
        assert_eq!(next_id_for(&EventTimeline::new()), 1);
        let mut lane = EventTimeline::new();
        lane.record(event(1.0, u64::MAX, EventType::Cue)).unwrap();
        assert_eq!(next_id_for(&lane), 1);
        lane.record(event(0.5, 4, EventType::Cue)).unwrap();
        assert_eq!(next_id_for(&lane), 5);
    }

    #[test]
    fn arming_then_clearing_then_undoing_returns_the_lane_and_the_allocator() {
        let mut lane = EventTimeline::new();
        let mut next = 1u64;
        let mut clear = ManualClear::new();
        record_into(&mut lane, &mut next, event(1.0, 1, EventType::Custom)).unwrap();
        record_into(&mut lane, &mut next, event(2.0, 9, EventType::Custom)).unwrap();
        assert_eq!(clear.button(), ClearButton::Clear);

        clear.arm();
        assert_eq!(clear.button(), ClearButton::Confirm);

        clear.clear(&mut lane, &mut next);
        assert!(lane.is_empty());
        assert_eq!(next, 1, "an empty lane can collide with nothing");
        assert_eq!(clear.button(), ClearButton::Undo);

        assert!(clear.undo(&mut lane, &mut next));
        assert_eq!(lane.len(), 2);
        assert_eq!(next, 10);
        assert_eq!(clear.button(), ClearButton::Clear);
        // Nothing left to give back.
        assert!(!clear.undo(&mut lane, &mut next));
    }

    #[test]
    fn a_recorded_event_retires_the_undo_offer() {
        let mut lane = EventTimeline::new();
        let mut next = 1u64;
        let mut clear = ManualClear::new();
        record_into(&mut lane, &mut next, event(1.0, 1, EventType::Custom)).unwrap();
        clear.arm();
        clear.clear(&mut lane, &mut next);
        assert_eq!(clear.button(), ClearButton::Undo);
        clear.forget();
        assert_eq!(clear.button(), ClearButton::Clear);
        assert!(!clear.undo(&mut lane, &mut next));
    }
}
