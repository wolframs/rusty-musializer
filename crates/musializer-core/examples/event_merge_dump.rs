//! Dumps the Rust merged event view for the same lane pairs as
//! `tests/differential/event_merge_oracle.c`.
//!
//! Run through `tools/differential_event_merge.sh`.

use musializer_core::scene::events::{
    EventRecord, SceneEventMerge, SEMANTIC_ID_LANE_BIT, TIMELINE_CAPACITY,
};

const LANE_BIT: u64 = SEMANTIC_ID_LANE_BIT;
/// The golden-ratio probe step. Duplicated from the C harness rather than read
/// from the library, so a wrong constant in the library still fails the diff.
const PROBE_STEP: u64 = 0x9E37_79B9_7F4A_7C15;

fn record(timestamp: f64, id: u64, event_type: u32) -> EventRecord {
    EventRecord {
        timestamp_seconds: timestamp,
        id,
        event_type,
        value_count: 1,
        values: [0.0; 4],
    }
}

fn dump(label: &str, manual: &[EventRecord], semantic: &[EventRecord]) {
    let mut merge = SceneEventMerge::new();
    match merge.build(manual, semantic) {
        Ok(()) => {
            let view = merge.view();
            println!("case {label} result 0 count {}", view.len());
            for (i, event) in view.events.iter().enumerate() {
                println!(
                    "{label} {i} {} {} {}",
                    g9(event.timestamp_seconds),
                    event.id,
                    event.event_type
                );
            }
        }
        Err(error) => {
            // The C returns an Event_Timeline_Result enum; only the OK path is
            // compared in detail, so a non-zero placeholder suffices here.
            println!("case {label} result nonzero({error}) count 0");
        }
    }
}

fn main() {
    dump("equal-ids", &[record(1.0, 7, 3)], &[record(2.0, 7, 2)]);
    dump("xor-to-zero", &[], &[record(1.0, LANE_BIT, 2)]);
    dump(
        "both-directions",
        &[],
        &[record(1.0, LANE_BIT | 5, 2), record(1.5, 5, 2)],
    );
    dump(
        "probe-once",
        &[record(0.5, 3 ^ LANE_BIT, 3)],
        &[record(1.0, 3, 2)],
    );
    dump(
        "probe-twice",
        &[
            record(0.5, 4 ^ LANE_BIT, 3),
            record(0.6, (4 ^ LANE_BIT).wrapping_add(PROBE_STEP), 3),
        ],
        &[record(1.0, 4, 2)],
    );
    // Each lane already canonical, as validate_lane requires; the semantic event
    // shares a timestamp with two manual ones and has type 2, so it must land
    // between the type-1 pair and the type-4 record.
    dump(
        "ordering",
        &[
            record(0.5, 100, 3),
            record(1.0, 5, 1),
            record(1.0, 9, 1),
            record(1.0, 1, 4),
        ],
        &[record(1.0, 42, 2)],
    );

    // The rejection cases. Each is a rule the first draft of this module missed.
    dump(
        "unsorted-lane",
        &[record(3.0, 1, 3), record(1.0, 2, 3)],
        &[],
    );
    dump("duplicate-id", &[record(1.0, 7, 3), record(2.0, 7, 3)], &[]);
    dump(
        "no-values",
        &[EventRecord {
            value_count: 0,
            ..record(1.0, 1, 3)
        }],
        &[],
    );
    dump("zero-id", &[record(1.0, 0, 3)], &[]);
    dump("unknown-type", &[record(1.0, 1, 99)], &[]);

    let manual: Vec<EventRecord> = (0..TIMELINE_CAPACITY as u64)
        .map(|i| record(i as f64 * 0.001, i + 1, 3))
        .collect();
    let semantic: Vec<EventRecord> = (0..TIMELINE_CAPACITY as u64)
        .map(|i| record(i as f64 * 0.001, i + 1, 2))
        .collect();
    let mut merge = SceneEventMerge::new();
    let result = merge.build(&manual, &semantic);
    println!(
        "case full-capacity result {} count {}",
        i32::from(result.is_err()),
        manual.len() + semantic.len()
    );
}

/// Formats like C's `%.9g`.
fn g9(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let exponent = value.abs().log10().floor() as i32;
    if (-5..9).contains(&exponent) {
        let precision = (8 - exponent).max(0) as usize;
        let s = format!("{value:.precision$}");
        return s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let s = format!("{value:.8e}");
    let (mantissa, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    format!("{mantissa}e{exp:+03}")
}
