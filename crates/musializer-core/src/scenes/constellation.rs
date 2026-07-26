//! Constellation: deterministic state, node geometry, and the authored-event lane.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_constellation.c`, with the
//! envelope filter in [`motion`] ported from `scene_constellation_motion.c/.h`.
//! Drawing lives in `musializer-app::scenes::constellation`.
//!
//! Constellation is the scene that reads **authored events**. That is a separate
//! evidence lane from measured audio and from model interpretation: an event's
//! `type` picks the flare colour outright (`:117-128`), so a manual cue and a
//! model-derived semantic event never render alike. The merged view the scene
//! reads has already namespaced semantic ids (`core::scene::events`), which is
//! what keeps two independently authored lanes with equal ids distinct here.

pub mod motion;

use std::any::Any;
use std::f32::consts::TAU;

use crate::scene::events::{EventRecord, EventType, MERGE_CAPACITY};
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

pub use motion::{Motion, MotionInput};

/// `scene_constellation.c:12` — the node budget at full density.
pub const NODE_COUNT: usize = 72;

/// A three-component position. See the note on the same type in
/// `spectral_terrarium`: core has no shared vector type yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// `scene_constellation.c:14-17`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConstellationState {
    pub seed: u64,
    pub motion: Motion,
}

/// `constellation_clamp01` (`:19-24`).
#[must_use]
pub fn clamp01(value: f32) -> f32 {
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `constellation_mix` (`:26-34`) — splitmix64's finalizer.
#[must_use]
pub fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

/// `constellation_unit` (`:36-40`).
#[must_use]
pub fn unit(seed: u64, salt: u64) -> f32 {
    (mix(seed ^ salt.wrapping_add(0x9e37_79b9_7f4a_7c15)) & 0xffff) as f32 / 65535.0
}

/// `constellation_band` (`:63-67`).
#[must_use]
pub fn band(frame: &SceneFrame<'_>, index: usize) -> f32 {
    let bands = frame.audio.bands;
    if bands.is_empty() {
        return 0.0;
    }
    clamp01(bands[index % bands.len()])
}

/// Nodes drawn at a given density setting (`:148-154`).
///
/// Density is a 1..3 integer slider, so the field is a third, two thirds, or all
/// of [`NODE_COUNT`]. The clamp is the C's, not a tidy-up.
#[must_use]
pub fn node_count(density: f32) -> usize {
    let density = (density.round() as i64).clamp(1, 3);
    NODE_COUNT * density as usize / 3
}

/// Keeps the event reach small enough that "near" and "far" stay distinguishable
/// around the ring (`:155`).
///
/// `saturating_sub` where the C subtracts freely: with the shipped density bounds
/// `node_count` is never below 24, so the C can never underflow here, but relying
/// on a setting range to keep an index calculation safe is not worth reproducing.
#[must_use]
pub fn clamp_event_reach(event_reach: usize, node_count: usize) -> usize {
    if event_reach * 2 >= node_count {
        (node_count / 2).saturating_sub(1)
    } else {
        event_reach
    }
}

/// The C's `event_record_is_valid` (`../musializer/src/event_timeline.c:32-44`).
///
/// Constellation *skips* an invalid event rather than drawing it, so the check has
/// to be the C's exactly: a zero id, an out-of-range type, or a zero value count
/// are all rejections, not clamps.
///
/// The integration owner is tightening `EventRecord::is_well_formed` in the shared
/// contract to the same rules, at which point this should collapse into a call to
/// it. It is kept as a local copy for now so the scene is correct against the C in
/// either version of the contract.
#[must_use]
pub fn event_is_valid(event: &EventRecord) -> bool {
    event.timestamp_seconds.is_finite()
        && event.timestamp_seconds >= 0.0
        && event.id != 0
        && event.event_type >= EventType::Lyric as u32
        && event.event_type <= EventType::Custom as u32
        && event.value_count > 0
        && (event.value_count as usize) <= crate::scene::events::VALUE_CAPACITY
        && event.values().iter().all(|value| value.is_finite())
}

/// The event that is lighting up one node, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventFlare {
    pub strength: f32,
    /// Raw event type. `0` means "no event", which is also what selects the
    /// neutral colour in [`event_hsv`].
    pub event_type: u32,
    pub event_id: u64,
}

/// `constellation_event_strength` (`:86-115`).
///
/// An event is hashed onto one node and decays exponentially with age; nodes
/// within `event_reach` of it around the ring get a third of the strength. The
/// strongest event wins, and its type and id come back with it because they pick
/// the colour and the displacement phase.
#[must_use]
pub fn event_flare(
    frame: &SceneFrame<'_>,
    node: usize,
    node_count: usize,
    event_duration: f32,
    event_reach: usize,
) -> EventFlare {
    let events = frame.events.events;
    if events.is_empty() || node_count == 0 {
        return EventFlare::default();
    }
    let count = events.len().min(MERGE_CAPACITY);
    let mut flare = EventFlare::default();
    for event in &events[..count] {
        if !event_is_valid(event) {
            continue;
        }
        let age = frame.time_seconds - event.timestamp_seconds;
        if age < 0.0 || age > f64::from(event_duration) {
            continue;
        }
        // The id *and* the type go into the hash, so two lanes that happen to
        // share an id still light up different nodes.
        let event_node =
            (mix(event.id ^ (u64::from(event.event_type) << 48)) % node_count as u64) as usize;
        let distance = node.abs_diff(event_node);
        if distance > event_reach && distance < node_count.saturating_sub(event_reach) {
            continue;
        }
        let payload = event.values[0].abs();
        let mut strength = (-(age as f32) * 2.1).exp() * (0.45 + clamp01(payload) * 0.55);
        if distance != 0 {
            strength *= 0.34;
        }
        if strength > flare.strength {
            flare = EventFlare {
                strength,
                event_type: event.event_type,
                event_id: event.id,
            };
        }
    }
    flare.strength = clamp01(flare.strength);
    flare
}

/// `constellation_base_position` (`:69-84`).
///
/// A Fibonacci sphere: nodes are spaced by the golden angle in longitude and
/// evenly in `y`, which distributes them without clumping at the poles.
#[must_use]
pub fn base_position(
    seed: u64,
    index: usize,
    node_count: usize,
    time: f32,
    amplitude: f32,
) -> Vec3 {
    let y = 1.0 - 2.0 * (index as f32 + 0.5) / node_count as f32;
    let radius = (1.0f32 - y * y).max(0.0).sqrt();
    let longitude = index as f32 * 2.399_963_2 + unit(seed, index as u64 * 3 + 1) * 0.28;
    let breathing = 1.0
        + amplitude * 0.18
        // The C writes the literal `6.2831853f` here rather than `2*PI`; it rounds
        // to the same `f32` as `TAU`, so this is bit-identical, not a tidy-up.
        + (time * 0.27 + unit(seed, index as u64 * 3 + 2) * TAU).sin() * 0.035;
    Vec3::new(
        longitude.cos() * radius * 3.8 * breathing,
        y * 3.15 * breathing,
        longitude.sin() * radius * 3.8 * breathing,
    )
}

/// Displacement applied to a node an event is lighting up (`:236-242`).
#[must_use]
pub fn event_displacement(flare: EventFlare, node: usize, energy: f32) -> Vec3 {
    if flare.strength <= 0.0 {
        return Vec3::default();
    }
    let phase = unit(flare.event_id, node as u64 + 31) * TAU;
    let displacement = flare.strength * (0.4 + energy * 0.55);
    Vec3::new(
        phase.cos() * displacement,
        (phase * 1.7).sin() * displacement,
        phase.sin() * displacement,
    )
}

/// `constellation_event_color` (`:117-128`), as an HSV triple so this stays in
/// core; the drawing half converts it with `runtime::draw::color_from_hsv`.
///
/// The fixed hues are the point: an authored lane is *identifiable*, not merely
/// present. Type `0` (no event) keeps the node's own spectral hue and desaturates.
#[must_use]
pub fn event_hsv(event_type: u32, hue: f32, brightness: f32) -> (f32, f32, f32) {
    let hue = match EventType::from_raw(event_type) {
        Some(EventType::Lyric) => 322.0,
        Some(EventType::Semantic) => 42.0,
        Some(EventType::Cue) => 164.0,
        Some(EventType::Custom) => 268.0,
        None => hue,
    };
    let saturation = if event_type == 0 { 0.48 } else { 0.82 };
    (hue, saturation, clamp01(brightness))
}

impl ConstellationState {
    /// `constellation_init` (`:42-48`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            motion: Motion::new(),
        }
    }
}

impl SceneState for ConstellationState {
    fn id(&self) -> SceneId {
        SceneId::Constellation
    }

    /// `constellation_update` (`:50-61`) — the whole update is the envelope filter.
    fn update(&mut self, frame: &SceneFrame<'_>) {
        self.motion.update(&MotionInput {
            time_seconds: frame.time_seconds,
            delta_seconds: frame.delta_seconds,
            rms: frame.audio.rms,
            spectral_flux: frame.audio.spectral_flux,
            onset: frame.audio.onset,
        });
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry entry (`scene_constellation_descriptor`, `:322-330`).
pub const DESCRIPTOR: SceneDescriptor = SceneDescriptor {
    id: SceneId::Constellation,
    state_version: 2,
    make_state: |seed| Box::new(ConstellationState::new(seed)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::events::EventTimelineView;
    use crate::scene::SceneSettings;

    /// The lane bit `core::scene::events` XORs into a semantic id
    /// (`SEMANTIC_ID_LANE_BIT`). Spelled out locally rather than imported so this
    /// test asserts what the *scene* needs — that two lanes stay distinct — without
    /// pinning the contract's spelling of the constant.
    const SEMANTIC_LANE_BIT: u64 = 1 << 63;

    fn event(timestamp: f64, id: u64, event_type: EventType, value: f32) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: event_type as u32,
            value_count: 1,
            values: [value, 0.0, 0.0, 0.0],
        }
    }

    fn frame_with_events<'a>(
        settings: &'a SceneSettings,
        time: f64,
        events: &'a [EventRecord],
    ) -> SceneFrame<'a> {
        SceneFrame {
            time_seconds: time,
            events: EventTimelineView { events },
            ..SceneFrame::idle(settings)
        }
    }

    #[test]
    fn density_selects_a_third_two_thirds_or_all_of_the_field() {
        assert_eq!(node_count(1.0), 24);
        assert_eq!(node_count(2.0), 48);
        assert_eq!(node_count(3.0), NODE_COUNT);
        // Out of range values clamp rather than producing an empty field.
        assert_eq!(node_count(0.0), 24);
        assert_eq!(node_count(99.0), NODE_COUNT);
        assert_eq!(node_count(2.4), 48, "rounds to nearest, as lroundf does");
    }

    #[test]
    fn nodes_lie_on_a_sphere_and_are_reproducible() {
        let count = node_count(3.0);
        for index in 0..count {
            let position = base_position(9, index, count, 0.0, 0.0);
            let radius =
                (position.x * position.x + position.y * position.y + position.z * position.z)
                    .sqrt();
            // 3.8 in the equatorial plane, 3.15 vertically, and the idle breathing
            // term is +/-3.5%, so those extremes bound the shell.
            assert!(
                (3.03..=3.94).contains(&radius),
                "node {index} sits at radius {radius}"
            );
            assert_eq!(position, base_position(9, index, count, 0.0, 0.0));
        }
        assert_ne!(
            base_position(9, 0, count, 0.0, 0.0),
            base_position(10, 0, count, 0.0, 0.0),
            "the seed perturbs longitude"
        );
    }

    #[test]
    fn an_event_lights_one_node_hardest_and_its_neighbours_faintly() {
        let settings = SceneSettings::default();
        let events = [event(1.0, 4242, EventType::Cue, 1.0)];
        let frame = frame_with_events(&settings, 1.0, &events);
        let count = node_count(3.0);
        let reach = clamp_event_reach(2, count);

        let flares: Vec<EventFlare> = (0..count)
            .map(|node| event_flare(&frame, node, count, 2.4, reach))
            .collect();
        let lit = flares.iter().filter(|flare| flare.strength > 0.0).count();
        assert_eq!(
            lit,
            reach * 2 + 1,
            "the hashed node plus `reach` on each side"
        );
        let strongest = flares
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.strength.total_cmp(&b.1.strength))
            .unwrap();
        assert_eq!(strongest.1.event_type, EventType::Cue as u32);
        assert_eq!(strongest.1.event_id, 4242);
        // The centre is roughly three times its neighbours (0.34 spill factor).
        for (node, flare) in flares.iter().enumerate() {
            if node != strongest.0 && flare.strength > 0.0 {
                assert!(flare.strength < strongest.1.strength * 0.4);
            }
        }
    }

    #[test]
    fn a_flare_decays_with_age_and_expires_at_the_setting() {
        let settings = SceneSettings::default();
        let events = [event(0.0, 7, EventType::Custom, 1.0)];
        let count = node_count(3.0);
        let node = (mix(7 ^ ((EventType::Custom as u64) << 48)) % count as u64) as usize;

        let fresh = event_flare(
            &frame_with_events(&settings, 0.0, &events),
            node,
            count,
            2.4,
            2,
        );
        let older = event_flare(
            &frame_with_events(&settings, 1.0, &events),
            node,
            count,
            2.4,
            2,
        );
        let expired = event_flare(
            &frame_with_events(&settings, 3.0, &events),
            node,
            count,
            2.4,
            2,
        );
        assert!(fresh.strength > older.strength);
        assert!(older.strength > 0.0);
        assert_eq!(expired.strength, 0.0, "past the event duration");

        // An event in the future does not reach back.
        let future = event_flare(
            &frame_with_events(&settings, -1.0, &events),
            node,
            count,
            2.4,
            2,
        );
        assert_eq!(future.strength, 0.0);
    }

    #[test]
    fn the_two_lanes_stay_distinct_even_with_equal_ids() {
        // `core::scene::events` qualifies semantic ids into their own lane on
        // merge; this asserts the scene actually benefits from it, by hashing them
        // to different nodes.
        let count = node_count(3.0);
        let manual = mix(9 ^ ((EventType::Cue as u64) << 48)) % count as u64;
        let semantic =
            mix((9 ^ SEMANTIC_LANE_BIT) ^ ((EventType::Semantic as u64) << 48)) % count as u64;
        assert_ne!(manual, semantic);

        // And the colours differ regardless of node.
        let (manual_hue, _, _) = event_hsv(EventType::Cue as u32, 200.0, 1.0);
        let (semantic_hue, _, _) = event_hsv(EventType::Semantic as u32, 200.0, 1.0);
        assert_eq!(manual_hue, 164.0);
        assert_eq!(semantic_hue, 42.0);
        // No event keeps the node's own hue and desaturates.
        let (own_hue, saturation, _) = event_hsv(0, 200.0, 1.0);
        assert_eq!(own_hue, 200.0);
        assert_eq!(saturation, 0.48);
    }

    #[test]
    fn malformed_events_are_skipped_the_way_the_c_skips_them() {
        let settings = SceneSettings::default();
        let count = node_count(3.0);
        // Zero id, unknown type, and zero value count are each rejected by the
        // C's event_record_is_valid, which is stricter than the shared contract.
        let bad = [
            EventRecord {
                timestamp_seconds: 0.0,
                id: 0,
                event_type: EventType::Cue as u32,
                value_count: 1,
                values: [1.0, 0.0, 0.0, 0.0],
            },
            EventRecord {
                timestamp_seconds: 0.0,
                id: 5,
                event_type: 99,
                value_count: 1,
                values: [1.0, 0.0, 0.0, 0.0],
            },
            EventRecord {
                timestamp_seconds: 0.0,
                id: 6,
                event_type: EventType::Cue as u32,
                value_count: 0,
                values: [1.0, 0.0, 0.0, 0.0],
            },
        ];
        for record in &bad {
            assert!(!event_is_valid(record), "{record:?} should be rejected");
        }
        let frame = frame_with_events(&settings, 0.0, &bad);
        for node in 0..count {
            assert_eq!(event_flare(&frame, node, count, 2.4, 2).strength, 0.0);
        }
    }

    #[test]
    fn the_update_only_advances_the_envelope_filter() {
        let settings = SceneSettings::default();
        let mut state = ConstellationState::new(5);
        let frame = SceneFrame {
            time_seconds: 1.0,
            delta_seconds: 1.0 / 60.0,
            ..SceneFrame::idle(&settings)
        };
        state.update(&frame);
        assert!(state.motion.initialized);
        assert_eq!(state.seed, 5, "the seed is never touched by an update");
    }

    #[test]
    fn the_descriptor_matches_the_c_registry_entry() {
        assert_eq!(DESCRIPTOR.id, SceneId::Constellation);
        assert_eq!(DESCRIPTOR.state_version, 2);
    }
}
