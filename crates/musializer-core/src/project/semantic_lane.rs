//! The model-derived semantic evidence lane.
//!
//! **Owner: Agent B.** Port of `../musializer/src/semantic_lane.c/.h`.
//!
//! This lane holds the latest model-derived semantic cue until the next one begins.
//! The measured audio frame stays separate, and that separation is the point:
//! callers decide how much creative influence to give an explicitly *interpretive*
//! lane, and they can only make that decision if it never gets mixed into the
//! measured one.
//!
//! Sampling deliberately **skips** malformed or out-of-range events instead of
//! failing (`semantic_lane.c:17-26`). An imported analysis artifact is third-party
//! evidence: one bad cue should cost that cue, not the whole lane.

use crate::scene::events::{EventTimelineView, EventType, TIMELINE_CAPACITY};

use super::event_timeline::record_is_valid;

/// The semantic values in force at one instant (`Semantic_Frame`,
/// `semantic_lane.h:9-16`).
///
/// `available` is false when no cue has started yet, in which case every value is
/// zero — but a caller should branch on `available` rather than reading zeros as
/// "neutral", because zero valence is a real value and zero confidence is not.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SemanticFrame {
    pub available: bool,
    /// The event id the values came from, for tracing a frame back to its evidence.
    pub source_id: u64,
    /// `0..=1`.
    pub energy: f32,
    /// `0..=1`.
    pub tension: f32,
    /// `-1..=1` — the only signed value in the lane.
    pub valence: f32,
    /// `0..=1`.
    pub confidence: f32,
}

/// `semantic_lane_sample` (`semantic_lane.c:6-38`).
///
/// Walks forward to `time_seconds` and keeps the last valid semantic cue that has
/// started, so a cue's values hold until the next one begins. Returns the frame and
/// whether anything was available, which is what C's `bool` return says.
///
/// `None` for a non-finite or negative time, or a view larger than the lane
/// capacity — the second is a corrupt caller, not a corrupt document.
#[must_use]
pub fn sample(events: EventTimelineView<'_>, time_seconds: f64) -> Option<SemanticFrame> {
    if !time_seconds.is_finite() || time_seconds < 0.0 || events.len() > TIMELINE_CAPACITY {
        return None;
    }
    let mut frame = SemanticFrame::default();
    for event in events.events {
        if event.timestamp_seconds > time_seconds {
            break;
        }
        if !record_is_valid(event)
            || event.event_type != EventType::Semantic as u32
            || event.value_count != 4
        {
            continue;
        }
        let [energy, tension, valence, confidence] = event.values;
        if !(0.0..=1.0).contains(&energy)
            || !(0.0..=1.0).contains(&tension)
            || !(-1.0..=1.0).contains(&valence)
            || !(0.0..=1.0).contains(&confidence)
        {
            continue;
        }
        frame = SemanticFrame {
            available: true,
            source_id: event.id,
            energy,
            tension,
            valence,
            confidence,
        };
    }
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::events::EventRecord;

    fn semantic(timestamp: f64, id: u64, values: [f32; 4]) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: EventType::Semantic as u32,
            value_count: 4,
            values,
        }
    }

    #[test]
    fn nothing_is_available_before_the_first_cue() {
        let events = [semantic(10.0, 1, [0.5, 0.5, 0.0, 0.9])];
        let frame = sample(EventTimelineView { events: &events }, 5.0).unwrap();
        assert!(!frame.available);
        assert_eq!(frame, SemanticFrame::default());
    }

    #[test]
    fn a_cues_values_hold_until_the_next_one_begins() {
        let events = [
            semantic(0.0, 1, [0.1, 0.2, -0.3, 0.4]),
            semantic(10.0, 2, [0.9, 0.8, 0.7, 0.6]),
        ];
        let view = EventTimelineView { events: &events };
        let first = sample(view, 5.0).unwrap();
        assert!(first.available);
        assert_eq!(first.source_id, 1);
        assert_eq!(first.energy, 0.1);
        assert_eq!(first.valence, -0.3);

        let second = sample(view, 10.0).unwrap();
        assert_eq!(second.source_id, 2, "the boundary belongs to the new cue");
        let later = sample(view, 9_999.0).unwrap();
        assert_eq!(later.source_id, 2, "and it holds indefinitely");
    }

    #[test]
    fn one_bad_cue_costs_only_that_cue() {
        // An imported artifact is third-party evidence: a single out-of-range cue
        // must not blank the lane.
        let events = [
            semantic(0.0, 1, [0.5, 0.5, 0.0, 0.5]),
            semantic(5.0, 2, [1.5, 0.5, 0.0, 0.5]),
            semantic(6.0, 3, [0.5, 0.5, -1.5, 0.5]),
            EventRecord {
                value_count: 3,
                ..semantic(7.0, 4, [0.5, 0.5, 0.0, 0.5])
            },
            EventRecord {
                event_type: EventType::Cue as u32,
                ..semantic(8.0, 5, [0.5, 0.5, 0.0, 0.5])
            },
            EventRecord {
                id: 0,
                ..semantic(9.0, 0, [0.5, 0.5, 0.0, 0.5])
            },
        ];
        let frame = sample(EventTimelineView { events: &events }, 100.0).unwrap();
        assert!(frame.available);
        assert_eq!(frame.source_id, 1, "the last *valid* cue still holds");
    }

    #[test]
    fn the_value_ranges_are_checked_at_their_endpoints() {
        for values in [[0.0, 0.0, -1.0, 0.0], [1.0, 1.0, 1.0, 1.0]] {
            let events = [semantic(0.0, 1, values)];
            let frame = sample(EventTimelineView { events: &events }, 1.0).unwrap();
            assert!(frame.available, "{values:?} is in range");
        }
        for values in [
            [-0.001, 0.0, 0.0, 0.0],
            [1.001, 0.0, 0.0, 0.0],
            [0.0, -0.001, 0.0, 0.0],
            [0.0, 0.0, -1.001, 0.0],
            [0.0, 0.0, 1.001, 0.0],
            [0.0, 0.0, 0.0, 1.001],
            [f32::NAN, 0.0, 0.0, 0.0],
        ] {
            let events = [semantic(0.0, 1, values)];
            let frame = sample(EventTimelineView { events: &events }, 1.0).unwrap();
            assert!(!frame.available, "{values:?} is out of range");
        }
    }

    #[test]
    fn an_unusable_time_yields_nothing_at_all() {
        let events = [semantic(0.0, 1, [0.5; 4])];
        let view = EventTimelineView { events: &events };
        assert_eq!(sample(view, -1.0), None);
        assert_eq!(sample(view, f64::NAN), None);
        assert_eq!(sample(view, f64::INFINITY), None);
    }

    #[test]
    fn an_empty_lane_samples_to_unavailable_not_an_error() {
        let frame = sample(EventTimelineView::EMPTY, 1.0).unwrap();
        assert!(!frame.available);
    }
}
