//! Loom: deterministic state, the woven record, and the cloth's structure.
//!
//! **Owner: Agent D.** Port of `../musializer/src/scene_loom.c`, with the woven
//! record in [`weave`] from `scene_loom_weave.c/.h`. Drawing lives in
//! `musializer-app::scenes::loom`.
//!
//! Loom is the scene that reads **accepted semantic energy, tension and valence**.
//! Those come from the model-interpretation lane, and [`semantic_at`] samples them
//! per *column* — each column is a fixed sample of the song at its own time, so
//! the finished tapestry is a record of the whole arc rather than of the current
//! frame. Where no lane covers a moment, the fallback is measured audio and it is
//! marked with a lower confidence, which keeps the two kinds of evidence
//! distinguishable in the picture.
//!
//! **The weave filling only part of the stage is not a framing bug.**
//! `../musializer/tools/UI_REVIEW.md` records this already: the cloth is revealed
//! in proportion to elapsed track time, so a capture at 15% of the track shows 15%
//! of the weave and that is correct.

pub mod weave;

use std::any::Any;

use crate::scene::events::{EventTimelineView, EventType, TIMELINE_CAPACITY, VALUE_CAPACITY};
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState, SemanticFrame};

pub use weave::{Weave, WeaveColumn, WeaveInput};

/// `scene_loom.c:6-12`
pub const MIN_COLUMNS: usize = 28;
pub const MAX_COLUMNS: usize = 144;
pub const MIN_ROWS: usize = 16;
pub const MAX_ROWS: usize = 72;
pub const WARP_SEGMENTS: usize = 32;

/// `scene_loom.c:14-17`
#[derive(Clone, Debug, PartialEq)]
pub struct LoomState {
    pub seed: u64,
    pub weave: Weave,
}

/// `loom_clamp01` (`:19-24`).
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

/// `loom_mix` (`:26-34`).
#[must_use]
pub fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value
}

/// `loom_unit` (`:36-39`) — like Cadence's, with no golden-ratio offset.
#[must_use]
pub fn unit(seed: u64, salt: u64) -> f32 {
    (mix(seed ^ salt) & 0xffff) as f32 / 65535.0
}

/// Column and row counts for a density setting (`:188-193`).
#[must_use]
pub fn dimensions(density_scale: f32) -> (usize, usize) {
    let columns = (72.0 * density_scale)
        .round()
        .clamp(MIN_COLUMNS as f32, MAX_COLUMNS as f32) as usize;
    let rows = (34.0 * density_scale)
        .round()
        .clamp(MIN_ROWS as f32, MAX_ROWS as f32) as usize;
    (columns, rows)
}

/// Spectral centroid of a coarse profile mapped to `[-1, 1]` (`:68-82`).
///
/// Bass-heavy moments read cool and treble-bright ones warm. **Only the
/// measured-audio fallback synthesizes valence this way**; a real semantic lane
/// keeps its own meaning, and conflating the two would collapse two evidence lanes.
#[must_use]
pub fn profile_valence(profile: &[f32; weave::BINS]) -> f32 {
    let mut total = 0.0f32;
    let mut weighted = 0.0f32;
    for (bin, &raw) in profile.iter().enumerate() {
        let value = clamp01(raw);
        total += value;
        weighted += value * bin as f32 / (weave::BINS - 1) as f32;
    }
    if total <= 0.0001 {
        return 0.0;
    }
    let valence = (weighted / total - 0.30) * 3.0;
    valence.clamp(-1.0, 1.0)
}

/// Port of `semantic_lane_sample` (`../musializer/src/semantic_lane.c`).
///
/// This is Agent B's module in the ownership map, and it is still a stub in
/// `core::project::semantic_lane`. Loom needs it now, so this is a local faithful
/// copy that should collapse into a call once B lands the real one — see the
/// Agent D note in REWRITE_PLAN.md.
///
/// Two details are reproduced deliberately rather than improved:
///
/// - Only a semantic event with **exactly four** in-range values counts. A
///   malformed or partial record is skipped, not clamped.
/// - A view longer than [`TIMELINE_CAPACITY`] is rejected outright, and the merged
///   view a scene receives can hold twice that (`MERGE_CAPACITY`). So a project
///   with more than 1024 merged events silently loses its semantic lane in Loom.
///   That looks like an oracle bug; it is reproduced and flagged, not fixed here.
#[must_use]
pub fn semantic_lane_sample(events: EventTimelineView<'_>, time_seconds: f64) -> SemanticFrame {
    let mut result = SemanticFrame::default();
    if !time_seconds.is_finite() || time_seconds < 0.0 || events.len() > TIMELINE_CAPACITY {
        return result;
    }
    for event in events.events {
        // Relies on the view's canonical ordering, exactly as the C does.
        if event.timestamp_seconds > time_seconds {
            break;
        }
        if !crate::scenes::constellation::event_is_valid(event)
            || event.event_type != EventType::Semantic as u32
            || event.value_count as usize != VALUE_CAPACITY
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
        result = SemanticFrame {
            available: true,
            source_id: event.id,
            energy,
            tension,
            valence,
            confidence,
        };
    }
    result
}

/// `loom_semantic_at` (`:84-109`) — the interpretation Loom weaves at one moment.
///
/// Precedence: an accepted semantic event, then the frame's own semantic lane, then
/// the measured-audio fallback. The fallback replays the envelope **frozen when the
/// fell passed** that slot, because asking for the same column time twice has to
/// keep answering the same thing or the tapestry stops being a record of the arc.
/// Its confidence (0.55 woven, 0.50 live) is what marks it as the weaker evidence.
#[must_use]
pub fn semantic_at(frame: &SceneFrame<'_>, weave: &Weave, time: f64) -> SemanticFrame {
    let sampled = semantic_lane_sample(frame.events, time);
    if sampled.available {
        return sampled;
    }
    if frame.semantic.available {
        return frame.semantic;
    }
    let column = &weave.columns[weave::slot(time, frame.duration_seconds)];
    let mut semantic = SemanticFrame {
        available: true,
        ..SemanticFrame::default()
    };
    if column.woven {
        semantic.energy = clamp01(column.energy);
        semantic.tension = clamp01(column.tension);
        semantic.valence = profile_valence(&column.profile);
        semantic.confidence = 0.55;
    } else {
        semantic.energy = clamp01(weave.energy);
        semantic.tension = clamp01(weave.tension);
        semantic.valence = profile_valence(&weave.profile);
        semantic.confidence = 0.50;
    }
    semantic
}

/// `loom_thread_color` (`:111-119`) as an HSV triple, so this stays in core.
///
/// Hue interpolates from 218 (cool, negative valence) to 28 (warm, positive).
#[must_use]
pub fn thread_hsv(
    semantic: SemanticFrame,
    saturation_scale: f32,
    brightness: f32,
) -> (f32, f32, f32) {
    let valence = clamp01((semantic.valence + 1.0) * 0.5);
    let hue = 218.0 + (28.0 - 218.0) * valence;
    let saturation = ((0.42 + semantic.tension * 0.38) * saturation_scale).min(1.0);
    (hue, saturation, clamp01(brightness))
}

/// Whether the warp passes over the weft at one crossing (`loom_warp_over`,
/// `:126-131`).
///
/// Weave topology is a structural consequence of the arc, not a tint: calm
/// passages interlace as plain weave, rising tension shifts to a 2/2 twill's
/// diagonal ribs, and peaks bind as a warp-faced satin whose long floats read
/// dense and glossy.
#[must_use]
pub fn warp_over(row: usize, column: usize, tension: f32) -> bool {
    if tension < 0.34 {
        return (row + column) & 1 == 0;
    }
    if tension < 0.67 {
        return (row + column * 2) & 3 < 2;
    }
    (row * 3 + column) % 5 != 0
}

/// `loom_crimp` (`:136-139`).
///
/// Threads pinch at each crossing and bulge between them; this is most of what
/// makes raster lines read as spun thread instead of wireframe.
#[must_use]
pub fn crimp(along: f32, crossings: f32) -> f32 {
    1.05 - 0.30 * (along * crossings * std::f32::consts::PI).sin().abs()
}

/// The band of cloth a thread belongs to (`loom_band_t`, `:143-146`).
///
/// 0 at the bottom edge (lowest frequencies) rising to 1 at the top, mirroring a
/// spectrogram laid on its side, so bass thickens the hem and highs shimmer along
/// the top selvage.
#[must_use]
pub fn band_t(y_t: f32) -> f32 {
    clamp01(1.0 - y_t)
}

impl LoomState {
    /// `loom_init` (`:41-46`).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            weave: Weave::new(),
        }
    }
}

impl SceneState for LoomState {
    fn id(&self) -> SceneId {
        SceneId::Loom
    }

    /// `loom_update` (`:48-63`).
    fn update(&mut self, frame: &SceneFrame<'_>) {
        self.weave.update(&WeaveInput {
            time_seconds: frame.time_seconds,
            duration_seconds: frame.duration_seconds,
            delta_seconds: frame.delta_seconds,
            bands: frame.audio.bands,
            rms: frame.audio.rms,
            spectral_flux: frame.audio.spectral_flux,
            onset: frame.audio.onset,
        });
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry entry (`scene_loom_descriptor`, `:391-399`).
pub const DESCRIPTOR: SceneDescriptor = SceneDescriptor {
    id: SceneId::Loom,
    state_version: 2,
    make_state: |seed| Box::new(LoomState::new(seed)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::events::EventRecord;
    use crate::scene::{SceneAudioFrame, SceneSettings};

    fn semantic_event(timestamp: f64, id: u64, values: [f32; 4]) -> EventRecord {
        EventRecord {
            timestamp_seconds: timestamp,
            id,
            event_type: EventType::Semantic as u32,
            value_count: 4,
            values,
        }
    }

    #[test]
    fn density_bounds_the_cloth_grid() {
        assert_eq!(dimensions(1.0), (72, 34));
        // The 0.50..2.00 setting range, and the hard bounds outside it.
        assert_eq!(dimensions(0.5), (MIN_COLUMNS + 8, 17));
        assert_eq!(dimensions(2.0), (144, 68));
        assert_eq!(dimensions(0.0), (MIN_COLUMNS, MIN_ROWS));
        assert_eq!(dimensions(99.0), (MAX_COLUMNS, MAX_ROWS));
    }

    #[test]
    fn the_weave_topology_changes_with_tension() {
        // Plain weave below 0.34: every crossing alternates.
        assert!(warp_over(0, 0, 0.1));
        assert!(!warp_over(0, 1, 0.1));
        // 2/2 twill between 0.34 and 0.67: pairs of floats.
        let twill: Vec<bool> = (0..8).map(|column| warp_over(0, column, 0.5)).collect();
        assert_eq!(
            twill,
            vec![true, false, true, false, true, false, true, false]
        );
        // Satin above 0.67: four floats in five, so the cloth reads dense.
        let satin = (0..20).filter(|&column| warp_over(0, column, 0.9)).count();
        assert_eq!(satin, 16);
    }

    #[test]
    fn crimp_pinches_at_crossings_and_bulges_between_them() {
        // At a crossing the sine is zero, so the thread is at its thickest.
        assert!((crimp(0.0, 10.0) - 1.05).abs() < 1.0e-6);
        // Halfway between, it pinches to 0.75.
        assert!((crimp(0.05, 10.0) - 0.75).abs() < 1.0e-5);
        for along in 0..100 {
            let value = crimp(along as f32 / 100.0, 34.0);
            assert!((0.74..=1.06).contains(&value));
        }
    }

    #[test]
    fn band_t_puts_bass_at_the_hem() {
        assert_eq!(
            band_t(1.0),
            0.0,
            "the bottom of the cloth is the lowest band"
        );
        assert_eq!(band_t(0.0), 1.0);
        assert_eq!(band_t(-1.0), 1.0, "clamped, not wrapped");
    }

    #[test]
    fn a_semantic_event_wins_over_the_measured_fallback() {
        let settings = SceneSettings::default();
        let events = [semantic_event(1.0, 3, [0.8, 0.6, 0.4, 0.9])];
        let frame = SceneFrame {
            duration_seconds: 100.0,
            events: EventTimelineView { events: &events },
            ..SceneFrame::idle(&settings)
        };
        let weave = Weave::new();
        let semantic = semantic_at(&frame, &weave, 2.0);
        assert!(semantic.available);
        assert_eq!(semantic.source_id, 3);
        assert_eq!(
            semantic.confidence, 0.9,
            "the lane's own confidence, not 0.55"
        );
        assert_eq!(semantic.energy, 0.8);
    }

    #[test]
    fn out_of_range_semantic_values_are_skipped_not_clamped() {
        let settings = SceneSettings::default();
        let events = [
            semantic_event(1.0, 3, [1.5, 0.6, 0.4, 0.9]),
            semantic_event(2.0, 4, [0.5, 0.6, -3.0, 0.9]),
            semantic_event(3.0, 5, [0.5, 0.6, 0.4, 2.0]),
        ];
        let frame = SceneFrame {
            duration_seconds: 100.0,
            events: EventTimelineView { events: &events },
            ..SceneFrame::idle(&settings)
        };
        assert!(!semantic_lane_sample(frame.events, 10.0).available);
        // With no lane, the fallback marks itself as the weaker evidence.
        let weave = Weave::new();
        let fallback = semantic_at(&frame, &weave, 10.0);
        assert!(fallback.available);
        assert_eq!(
            fallback.confidence, 0.50,
            "live fallback, nothing woven yet"
        );
        assert_eq!(fallback.source_id, 0, "the fallback claims no source");
    }

    #[test]
    fn the_woven_fallback_answers_the_same_thing_twice() {
        let settings = SceneSettings::default();
        let bands: Vec<f32> = (0..64).map(|i| if i < 32 { 0.9 } else { 0.1 }).collect();
        let trails = vec![0.0f32; 64];
        let mut state = LoomState::new(1);
        for i in 0..300u32 {
            let mut audio = SceneAudioFrame::from_spectrum(&bands, &trails);
            audio.rms = 0.5;
            audio.spectral_flux = 0.1;
            let frame = SceneFrame {
                time_seconds: f64::from(i) / 60.0,
                duration_seconds: 14.4,
                delta_seconds: if i == 0 { 0.0 } else { 1.0 / 60.0 },
                audio,
                ..SceneFrame::idle(&settings)
            };
            state.update(&frame);
        }
        let frame = SceneFrame {
            time_seconds: 5.0,
            duration_seconds: 14.4,
            ..SceneFrame::idle(&settings)
        };
        let first = semantic_at(&frame, &state.weave, 1.0);
        let second = semantic_at(&frame, &state.weave, 1.0);
        // Compared field-by-field because `SemanticFrame` in the shared contract
        // does not derive `PartialEq` yet; see the Agent D note in REWRITE_PLAN.md.
        assert_eq!(
            (first.energy, first.tension, first.valence, first.confidence),
            (
                second.energy,
                second.tension,
                second.valence,
                second.confidence
            )
        );
        assert_eq!(
            first.confidence, 0.55,
            "a woven slot, not the live envelope"
        );
        assert!(first.energy > 0.0, "the woven slot recorded real energy");
    }

    #[test]
    fn profile_valence_reads_bass_as_cool_and_treble_as_warm() {
        let mut bass_only = [0.0f32; weave::BINS];
        bass_only[0] = 1.0;
        let mut treble_only = [0.0f32; weave::BINS];
        treble_only[weave::BINS - 1] = 1.0;
        assert!(
            (profile_valence(&bass_only) - -0.9).abs() < 1.0e-5,
            "a centroid of 0 maps to -0.9"
        );
        assert_eq!(
            profile_valence(&treble_only),
            1.0,
            "clamped at the warm end"
        );
        // Silence carries no opinion rather than defaulting to one end.
        assert_eq!(profile_valence(&[0.0; weave::BINS]), 0.0);
        // The neutral point is a centroid of 0.30, not 0.5 — a deliberate bias in
        // the oracle toward reading ordinary music as slightly cool.
        let mut neutral = [0.0f32; weave::BINS];
        neutral[((weave::BINS - 1) as f32 * 0.30).round() as usize] = 1.0;
        assert!(profile_valence(&neutral).abs() < 0.05);
    }

    #[test]
    fn thread_hue_runs_cool_to_warm_with_valence() {
        let cool = thread_hsv(
            SemanticFrame {
                valence: -1.0,
                ..SemanticFrame::default()
            },
            1.0,
            1.0,
        );
        let warm = thread_hsv(
            SemanticFrame {
                valence: 1.0,
                ..SemanticFrame::default()
            },
            1.0,
            1.0,
        );
        assert!((cool.0 - 218.0).abs() < 1.0e-4);
        assert!((warm.0 - 28.0).abs() < 1.0e-4);
        // Saturation is capped at 1 however high the scale goes.
        let saturated = thread_hsv(
            SemanticFrame {
                tension: 1.0,
                ..SemanticFrame::default()
            },
            1.3,
            1.0,
        );
        assert_eq!(saturated.1, 1.0);
    }

    #[test]
    fn a_merged_view_larger_than_the_lane_capacity_loses_the_lane() {
        // Reproduces a suspected oracle bug rather than fixing it: the merged view
        // holds up to 2 * TIMELINE_CAPACITY, and semantic_lane_sample rejects
        // anything longer than TIMELINE_CAPACITY outright.
        let event = semantic_event(0.0, 1, [0.5, 0.5, 0.0, 1.0]);
        let fits = vec![event; TIMELINE_CAPACITY];
        let too_many = vec![event; TIMELINE_CAPACITY + 1];
        assert!(semantic_lane_sample(EventTimelineView { events: &fits }, 1.0).available);
        assert!(!semantic_lane_sample(EventTimelineView { events: &too_many }, 1.0).available);
    }

    #[test]
    fn the_descriptor_matches_the_c_registry_entry() {
        assert_eq!(DESCRIPTOR.id, SceneId::Loom);
        assert_eq!(DESCRIPTOR.state_version, 2);
    }
}
