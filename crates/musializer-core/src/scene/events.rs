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

    /// The lane colour for this kind (`event_type_color`, `plug.c:1521-1530`).
    ///
    /// Packed `0xRRGGBBAA` and raylib-free for the same reason the workspace
    /// palette is (`ui_palette.h`'s header note): a colour that exists only as a
    /// raylib `Color` is invisible to a contrast sweep. These live *here* rather
    /// than in the application's `theme` because they identify an **event type**,
    /// not a piece of chrome — the manual event row, the timeline markers and the
    /// lyric lane all have to agree about what colour a cue is, and the only
    /// thing all three share is this enum.
    #[must_use]
    pub const fn rgba(self) -> u32 {
        match self {
            EventType::Lyric => 0xEC59_BEFF,
            EventType::Semantic => 0xF2BE_42FF,
            EventType::Cue => 0x3FDC_ABFF,
            EventType::Custom => 0x976F_F1FF,
        }
    }
}

/// The colour an unrecognized event type draws in: raylib's `GRAY`, which is the
/// C's `default:` arm (`plug.c:1528`).
pub const UNKNOWN_EVENT_RGBA: u32 = 0x8282_82FF;

/// One recorded event (`event_timeline.h:21-28`).
///
/// C's `reserved[3]` padding is omitted: it exists to make the C struct's layout
/// explicit, and Rust has no equivalent need here since nothing serializes this
/// struct byte-wise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventRecord {
    pub timestamp_seconds: f64,
    /// Must be non-zero: `0` means "no event" (`event_timeline.c:35`).
    pub id: u64,
    /// A raw `u32` because that is the C field's type, but only `1..=4` is
    /// **valid** — the oracle rejects an unknown type rather than carrying it
    /// through (`event_timeline.c:36`). Use [`EventRecord::kind`] to resolve it.
    pub event_type: u32,
    /// Must be `1..=4`. A record with **no** values is invalid
    /// (`event_timeline.c:37`).
    pub value_count: u8,
    pub values: [f32; VALUE_CAPACITY],
}

impl Default for EventRecord {
    /// Note: the default record is **not** valid — `id` is 0, `event_type` is 0,
    /// and `value_count` is 0, all of which the oracle rejects. It exists for
    /// struct-update syntax, not as a usable event.
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
    /// The values this event carries.
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values[..(self.value_count as usize).min(VALUE_CAPACITY)]
    }

    #[must_use]
    pub fn kind(&self) -> Option<EventType> {
        EventType::from_raw(self.event_type)
    }

    /// Exactly `event_record_is_valid` (`event_timeline.c:32-44`).
    ///
    /// Four of these rules are easy to omit and all four were, in the first draft
    /// of this module: a **zero id is invalid**, a **zero `value_count` is
    /// invalid**, an **unknown `event_type` is invalid** (it is not carried
    /// through), and the timestamp must be non-negative as well as finite.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.timestamp_seconds.is_finite()
            && self.timestamp_seconds >= 0.0
            && self.id != 0
            && self.kind().is_some()
            && self.value_count >= 1
            && (self.value_count as usize) <= VALUE_CAPACITY
            && self.values().iter().all(|value| value.is_finite())
    }

    /// The canonical sort key: `(timestamp, type, id)`
    /// (`event_compare`, `scene_event_merge.c:6-17`).
    fn sort_key(&self) -> (f64, u32, u64) {
        (self.timestamp_seconds, self.event_type, self.id)
    }

    fn compare(&self, other: &Self) -> std::cmp::Ordering {
        let (at, atype, aid) = self.sort_key();
        let (bt, btype, bid) = other.sort_key();
        at.total_cmp(&bt)
            .then_with(|| atype.cmp(&btype))
            .then_with(|| aid.cmp(&bid))
    }
}

/// Validates one lane the way `event_timeline_validate` does
/// (`event_timeline.c:46-65`).
///
/// A timeline must be **strictly increasing** by `(timestamp, type, id)` and every
/// id must be unique across the whole lane. Equal consecutive keys are an error,
/// reported as a duplicate id when the ids match and as an ordering error
/// otherwise.
///
/// # Errors
/// See [`EventTimelineError`].
pub fn validate_lane(events: &[EventRecord]) -> Result<(), EventTimelineError> {
    if events.len() > TIMELINE_CAPACITY {
        return Err(EventTimelineError::Overflow);
    }
    for (i, event) in events.iter().enumerate() {
        if !event.is_well_formed() {
            return Err(EventTimelineError::Malformed);
        }
        if i > 0 && events[i - 1].compare(event) != std::cmp::Ordering::Less {
            return Err(if events[i - 1].id == event.id {
                EventTimelineError::DuplicateId
            } else {
                EventTimelineError::Order
            });
        }
        if events[..i].iter().any(|earlier| earlier.id == event.id) {
            return Err(EventTimelineError::DuplicateId);
        }
    }
    Ok(())
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

/// The bit XORed into a semantic id to move it into its own display namespace
/// (`../musializer/src/scene_event_merge.c:31`).
///
/// **XOR, not OR.** XOR is a one-to-one lane transform, so two distinct semantic
/// ids can never collapse onto one qualified id. OR is not injective — it maps
/// every id that already has the high bit set onto itself — which would silently
/// merge two events. This distinction is the whole reason the C comment calls it
/// "a one-to-one lane transform".
pub const SEMANTIC_ID_LANE_BIT: u64 = 0x8000_0000_0000_0000;

/// Step used to probe past a collision (`scene_event_merge.c:34`).
///
/// The 64-bit golden-ratio constant. Any odd stride would terminate; this one is
/// conventional and spreads probes well.
const COLLISION_PROBE_STEP: u64 = 0x9E37_79B9_7F4A_7C15;

/// Moves a semantic id into the semantic lane's namespace, avoiding zero and any
/// id already present (`qualify_semantic_id`, `scene_event_merge.c:27-38`).
///
/// The bounded probe exists for one narrow case the C comment names: an *authored*
/// manual id that already occupies the transformed value, or a value a previous
/// probe took. Without it, a manual event and a semantic event could share an id
/// in the merged view, which is precisely what namespacing is meant to prevent.
///
/// Zero is skipped because an id of 0 means "no event" elsewhere in the model.
fn qualify_semantic_id(existing: &[EventRecord], id: u64) -> u64 {
    let mut qualified = id ^ SEMANTIC_ID_LANE_BIT;
    if qualified == 0 {
        qualified = SEMANTIC_ID_LANE_BIT | 1;
    }
    while existing.iter().any(|event| event.id == qualified) {
        qualified = qualified.wrapping_add(COLLISION_PROBE_STEP);
        if qualified == 0 {
            qualified = 1;
        }
    }
    qualified
}

/// Which authored lane a merged event came from.
///
/// **Not derivable from the merged record, which is the whole reason this
/// exists.** The obvious shortcut — test [`SEMANTIC_ID_LANE_BIT`] on the id — is
/// wrong twice over. `qualify_semantic_id` adds [`COLLISION_PROBE_STEP`] on a
/// collision, which lands anywhere in the 64-bit space and clears the bit as
/// often as not; and a *manual* id is carried through untouched, so a manual
/// event authored with the high bit already set is indistinguishable from a
/// qualified semantic one. Type does not answer it either: the manual event row's
/// `+ Feel` button records [`EventType::Semantic`] into the **manual** lane
/// (`plug.c:2897`, `ui/panels/events.rs`'s `MARKERS`), so the two axes genuinely
/// cross.
///
/// Carried alongside the records rather than inside them so the merged view stays
/// byte-identical to the oracle's: `differential_event_merge.sh` pins the ids and
/// the ordering, and this adds a lane nobody has to serialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventLane {
    /// Authored by hand — the manual event row, or a `.musi` file's manual lane.
    Manual,
    /// Derived by the model — the semantic lane, whose ids were qualified.
    Semantic,
}

impl EventLane {
    /// A short human name, for a legend or a tooltip.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            EventLane::Manual => "manual",
            EventLane::Semantic => "semantic",
        }
    }
}

/// The merged, canonically ordered view of the manual and semantic lanes.
#[derive(Clone, Debug, Default)]
pub struct SceneEventMerge {
    events: Vec<EventRecord>,
    /// Parallel to `events`: `lanes[i]` is the authored lane of `events[i]`.
    lanes: Vec<EventLane>,
}

impl SceneEventMerge {
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            lanes: Vec::new(),
        }
    }

    /// Builds one deterministic, canonically ordered view of both lanes.
    ///
    /// Port of `scene_event_merge_build` (`scene_event_merge.c:40-69`). Manual ids
    /// are preserved; semantic ids go through [`qualify_semantic_id`].
    ///
    /// Order is `(timestamp, type, id)` — **`type` is part of the key**, between
    /// the other two (`event_compare`, `scene_event_merge.c:6-17`). Dropping it
    /// would reorder two events that share a timestamp, which changes what a scene
    /// draws for that frame.
    ///
    /// Semantic events are qualified in input order against the partially built
    /// result, so each probe sees the manual lane plus every semantic id already
    /// placed. That ordering is load-bearing: qualifying against the finished set
    /// instead would give different ids.
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
        // C validates each lane in full — not merely each record — and only then
        // checks the combined count against capacity, in that order
        // (`scene_event_merge.c:48-53`). The order is observable: an invalid
        // manual lane reports its own error rather than an overflow.
        validate_lane(manual)?;
        validate_lane(semantic)?;
        if manual.len() + semantic.len() > MERGE_CAPACITY {
            return Err(EventTimelineError::Overflow);
        }

        self.events.clear();
        self.lanes.clear();
        self.events.extend_from_slice(manual);
        self.lanes.resize(manual.len(), EventLane::Manual);
        for event in semantic {
            let id = qualify_semantic_id(&self.events, event.id);
            self.events.push(EventRecord { id, ..*event });
            self.lanes.push(EventLane::Semantic);
        }
        // Sorted as a permutation rather than by sorting `events` directly, so
        // `lanes` follows its record instead of silently going stale. The
        // comparator and the stability are unchanged, so the resulting order is
        // the same one `differential_event_merge.sh` pins.
        //
        // `total_cmp` rather than `partial_cmp` because the sort must be total;
        // malformed timestamps were already rejected above.
        let mut order: Vec<usize> = (0..self.events.len()).collect();
        order
            .sort_by(|&left, &right| EventRecord::compare(&self.events[left], &self.events[right]));
        let sorted_events: Vec<EventRecord> = order.iter().map(|&i| self.events[i]).collect();
        let sorted_lanes: Vec<EventLane> = order.iter().map(|&i| self.lanes[i]).collect();
        self.events = sorted_events;
        self.lanes = sorted_lanes;
        Ok(())
    }

    #[must_use]
    pub fn view(&self) -> EventTimelineView<'_> {
        EventTimelineView {
            events: &self.events,
        }
    }

    /// The authored lane of each merged event, parallel to [`Self::view`].
    ///
    /// Same length and same order as `view().events`, which is asserted below
    /// rather than merely promised.
    #[must_use]
    pub fn lanes(&self) -> &[EventLane] {
        debug_assert_eq!(self.lanes.len(), self.events.len());
        &self.lanes
    }

    /// The merged events paired with the lane each came from.
    pub fn iter_with_lane(&self) -> impl Iterator<Item = (&EventRecord, EventLane)> + '_ {
        self.events.iter().zip(self.lanes.iter().copied())
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.lanes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A *valid* event: non-zero id, known type, at least one finite value.
    fn event(timestamp: f64, id: u64, event_type: EventType) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: event_type as u32,
            value_count: 1,
            values: [0.0; VALUE_CAPACITY],
        }
    }

    #[test]
    fn event_type_discriminants_start_at_one() {
        assert_eq!(EventType::Lyric as u32, 1);
        assert_eq!(EventType::Custom as u32, 4);
        assert_eq!(EventType::from_raw(0), None);
        assert_eq!(EventType::from_raw(5), None);
    }

    /// The first draft of this module carried an unknown type through on the
    /// theory that dropping it would be worse. The oracle disagrees: an unknown
    /// type is malformed (`event_timeline.c:36`). Inventing tolerance the oracle
    /// does not have is a parity bug, so this pins the real rule.
    #[test]
    fn an_unknown_event_type_is_rejected_not_carried() {
        let record = EventRecord {
            event_type: 99,
            ..event(1.0, 1, EventType::Cue)
        };
        assert_eq!(record.kind(), None);
        assert!(!record.is_well_formed());
    }

    #[test]
    fn the_default_record_is_deliberately_invalid() {
        // id 0, type 0 and value_count 0 are each independently rejected.
        assert!(!EventRecord::default().is_well_formed());
    }

    #[test]
    fn a_zero_id_and_an_empty_value_list_are_both_invalid() {
        let valid = event(1.0, 5, EventType::Cue);
        assert!(valid.is_well_formed());
        assert!(!EventRecord { id: 0, ..valid }.is_well_formed());
        assert!(!EventRecord {
            value_count: 0,
            ..valid
        }
        .is_well_formed());
        assert!(!EventRecord {
            value_count: (VALUE_CAPACITY + 1) as u8,
            ..valid
        }
        .is_well_formed());
    }

    #[test]
    fn a_lane_must_be_strictly_sorted_with_unique_ids() {
        let a = event(1.0, 1, EventType::Cue);
        let b = event(2.0, 2, EventType::Cue);
        validate_lane(&[a, b]).expect("sorted and unique");

        assert_eq!(
            validate_lane(&[b, a]).unwrap_err(),
            EventTimelineError::Order,
            "out of order"
        );
        assert_eq!(
            validate_lane(&[a, a]).unwrap_err(),
            EventTimelineError::DuplicateId,
            "an identical consecutive record reports the duplicate id, not the order"
        );
        // Same id at a later timestamp: ordering is fine, the id is not.
        let same_id_later = EventRecord {
            timestamp_seconds: 3.0,
            ..a
        };
        assert_eq!(
            validate_lane(&[a, same_id_later]).unwrap_err(),
            EventTimelineError::DuplicateId
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
        assert_eq!(view.events[1].id, 7 ^ SEMANTIC_ID_LANE_BIT);
        assert_ne!(view.events[0].id, view.events[1].id);
    }

    /// The lane travels with its record through the sort, which is the only thing
    /// that makes a marker's colour honest: the merge interleaves both lanes by
    /// timestamp, so `lanes[i]` and `events[i]` are re-paired on every build.
    #[test]
    fn the_lane_follows_its_record_through_the_merge_sort() {
        // Interleaved on purpose: manual at 1 and 3, semantic at 2 and 4, so a
        // lane list that was not permuted with the records would be visibly wrong.
        let manual = [event(1.0, 1, EventType::Cue), event(3.0, 2, EventType::Cue)];
        let semantic = [
            event(2.0, 1, EventType::Semantic),
            event(4.0, 2, EventType::Semantic),
        ];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &semantic).unwrap();

        let observed: Vec<(f64, EventLane)> = merge
            .iter_with_lane()
            .map(|(event, lane)| (event.timestamp_seconds, lane))
            .collect();
        assert_eq!(
            observed,
            vec![
                (1.0, EventLane::Manual),
                (2.0, EventLane::Semantic),
                (3.0, EventLane::Manual),
                (4.0, EventLane::Semantic),
            ]
        );
        assert_eq!(merge.lanes().len(), merge.view().len());
    }

    /// The shortcut this module exists to make unnecessary, pinned as a negative
    /// so nobody reintroduces it. Both halves fail: a **manual** id may carry the
    /// lane bit already, and a *qualified semantic* id may not carry it after a
    /// collision probe.
    #[test]
    fn the_lane_bit_cannot_stand_in_for_the_lane() {
        // A manual event authored with the high bit set. Nothing forbids it: the
        // manual lane's ids are carried through untouched.
        let manual = [event(1.0, SEMANTIC_ID_LANE_BIT | 9, EventType::Cue)];
        // Its semantic partner qualifies to `SEMANTIC_ID_LANE_BIT | 9` too, which
        // the manual event already occupies — so the collision probe fires and
        // lands on a value with the lane bit **clear**. That is the second half of
        // the point, and it arrived from running the test rather than from
        // predicting it: the probe is not a rare path, it is one XOR collision away.
        let semantic = [event(2.0, 9, EventType::Semantic)];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &semantic).unwrap();

        let pairs: Vec<(u64, EventLane)> = merge
            .iter_with_lane()
            .map(|(event, lane)| (event.id, lane))
            .collect();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], (SEMANTIC_ID_LANE_BIT | 9, EventLane::Manual));
        assert_eq!(pairs[1].1, EventLane::Semantic);
        assert_eq!(
            pairs[1].0,
            (SEMANTIC_ID_LANE_BIT | 9).wrapping_add(COLLISION_PROBE_STEP),
            "the probe stepped once past the manual id"
        );
        assert_eq!(
            pairs[1].0 & SEMANTIC_ID_LANE_BIT,
            0,
            "and the step cleared the lane bit"
        );
        for (id, lane) in pairs {
            let guessed = if id & SEMANTIC_ID_LANE_BIT != 0 {
                EventLane::Semantic
            } else {
                EventLane::Manual
            };
            assert_ne!(guessed, lane, "the bit test is backwards for id {id:#x}");
        }
    }

    /// Type does not answer it either. `+ Feel` records `EventType::Semantic`
    /// into the **manual** lane (`plug.c:2897`), so the two axes cross and a
    /// marker coloured only by type cannot say which lane it came from.
    #[test]
    fn a_manual_event_may_carry_the_semantic_type() {
        let manual = [event(1.0, 1, EventType::Semantic)];
        let semantic = [event(2.0, 1, EventType::Semantic)];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &semantic).unwrap();
        let lanes: Vec<EventLane> = merge.lanes().to_vec();
        assert_eq!(lanes, vec![EventLane::Manual, EventLane::Semantic]);
        // Same type, different lane.
        assert_eq!(merge.view().events[0].kind(), merge.view().events[1].kind());
    }

    /// XOR is one-to-one; OR is not. An id that already has the lane bit set must
    /// come *back* across the boundary rather than mapping onto itself, otherwise
    /// two distinct semantic ids could collapse onto one.
    #[test]
    fn qualification_is_one_to_one_not_a_bitwise_or() {
        let already_set = SEMANTIC_ID_LANE_BIT | 5;
        assert_eq!(qualify_semantic_id(&[], already_set), 5);
        assert_eq!(qualify_semantic_id(&[], 5), already_set);
        // The distinctness that OR would have destroyed.
        assert_ne!(
            qualify_semantic_id(&[], already_set),
            qualify_semantic_id(&[], 5)
        );
    }

    #[test]
    fn qualification_never_produces_zero() {
        // Exactly the lane bit XORs to 0, which means "no event" elsewhere.
        assert_eq!(
            qualify_semantic_id(&[], SEMANTIC_ID_LANE_BIT),
            SEMANTIC_ID_LANE_BIT | 1
        );
    }

    #[test]
    fn qualification_probes_past_an_authored_id_that_already_took_the_slot() {
        // A manual event has authored the very id a semantic id would transform
        // into. The probe must step past it, not duplicate it.
        let taken = 3 ^ SEMANTIC_ID_LANE_BIT;
        let manual = [EventRecord {
            id: taken,
            ..Default::default()
        }];
        let qualified = qualify_semantic_id(&manual, 3);
        assert_ne!(qualified, taken);
        assert_eq!(qualified, taken.wrapping_add(COLLISION_PROBE_STEP));
    }

    /// `event_compare` sorts by `(timestamp, type, id)`. Omitting `type` would
    /// reorder two events sharing a timestamp, changing what a scene draws.
    ///
    /// Both lanes must arrive already sorted, so the cross-lane interleave is
    /// where the merge's own comparison shows: a semantic `Semantic` (type 2)
    /// event must sort ahead of a manual `Cue` (type 3) at the same timestamp,
    /// regardless of their ids.
    #[test]
    fn ordering_uses_type_between_timestamp_and_id() {
        let manual = [event(1.0, 1, EventType::Cue)];
        let semantic = [event(1.0, 9, EventType::Semantic)];
        let mut merge = SceneEventMerge::new();
        merge.build(&manual, &semantic).unwrap();
        let view = merge.view();
        assert_eq!(
            view.events[0].event_type,
            EventType::Semantic as u32,
            "type 2 sorts ahead of type 3 even though its id is larger"
        );
        assert_eq!(view.events[1].event_type, EventType::Cue as u32);
        // And within one type, id breaks the tie.
        let manual = [event(2.0, 1, EventType::Cue), event(2.0, 2, EventType::Cue)];
        merge.build(&manual, &[]).unwrap();
        assert_eq!(merge.view().events[0].id, 1);
        assert_eq!(merge.view().events[1].id, 2);
    }

    #[test]
    fn merging_interleaves_the_lanes_and_is_deterministic() {
        // Each lane sorted, as validate_lane requires; the merge interleaves them.
        let manual = [event(1.0, 2, EventType::Cue), event(3.0, 1, EventType::Cue)];
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
    fn an_unsorted_input_lane_is_rejected() {
        // The merge does not sort its inputs into shape; each lane is already
        // required to be canonical, and a caller that got that wrong hears about
        // it rather than getting a quietly reordered view.
        let manual = [event(3.0, 1, EventType::Cue), event(1.0, 2, EventType::Cue)];
        let mut merge = SceneEventMerge::new();
        assert_eq!(
            merge.build(&manual, &[]).unwrap_err(),
            EventTimelineError::Order
        );
    }

    #[test]
    fn both_lanes_fit_at_full_size() {
        // Distinct ids and increasing timestamps, so each lane is itself valid.
        let lane = |base: u64| -> Vec<EventRecord> {
            (0..TIMELINE_CAPACITY as u64)
                .map(|i| event(i as f64 * 0.001, base + i, EventType::Cue))
                .collect()
        };
        let manual = lane(1);
        let semantic = lane(1);
        let mut merge = SceneEventMerge::new();
        merge
            .build(&manual, &semantic)
            .expect("the merged view must carry both lanes at full size");
        assert_eq!(merge.view().len(), MERGE_CAPACITY);
        // Namespacing kept all 2048 ids distinct despite both lanes using 1..=1024.
        let unique: std::collections::HashSet<u64> =
            merge.view().events.iter().map(|e| e.id).collect();
        assert_eq!(unique.len(), MERGE_CAPACITY);

        // One record past a single lane's capacity is an overflow.
        let too_many: Vec<EventRecord> = (0..=TIMELINE_CAPACITY as u64)
            .map(|i| event(i as f64 * 0.001, i + 1, EventType::Cue))
            .collect();
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
