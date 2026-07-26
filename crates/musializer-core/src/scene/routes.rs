//! Audio-to-parameter routes: the shared modulation layer under all ten scenes.
//!
//! **Shared contract.** Consumed by Agents B, C, D and F. Port of
//! `../musializer/src/scene_routes.c`/`.h` and the mapping evaluation in
//! `project.c:516-566`.
//!
//! A route replaces its target setting's value each frame with a mapped audio
//! source, clamped to the setting descriptor's range. Sliders are the degenerate
//! constant case. Routes are **user-authored configuration, never analysis
//! output** (`scene_routes.h:10-15`) — which is why they live beside settings
//! rather than in an evidence lane.
//!
//! Agent B owns the `.musi` codec that persists these. The evaluation semantics
//! are here so the frame loop, the Tune editor's live readout, and the transfer
//! graph all call one function and cannot drift — which is the reason C factored
//! `scene_route_output_value` out in the first place (`scene_routes.h:99-103`).

use super::settings::{self, SceneSettings, SettingDescriptor, SettingKind, MAX_CONTROLS};
use super::SceneId;

/// Routes one scene may carry, equal to its control count cap
/// (`scene_routes.h:16`).
pub const ROUTES_PER_SCENE: usize = MAX_CONTROLS;

/// Which per-frame audio figure drives a route (`project.h:60-67`).
///
/// Discriminants are persisted in `.musi`, so they are a compatibility surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AnalysisSource {
    Rms = 0,
    Peak = 1,
    SpectralFlux = 2,
    BeatPhase = 3,
    /// One analyzer band, selected by `band_index`.
    Band = 4,
}

impl AnalysisSource {
    pub const ALL: [AnalysisSource; 5] = [
        AnalysisSource::Rms,
        AnalysisSource::Peak,
        AnalysisSource::SpectralFlux,
        AnalysisSource::BeatPhase,
        AnalysisSource::Band,
    ];

    /// The codec's canonical name, as `--route` specs and `.musi` files spell it.
    #[must_use]
    pub fn canonical_name(self) -> &'static str {
        match self {
            AnalysisSource::Rms => "rms",
            AnalysisSource::Peak => "peak",
            AnalysisSource::SpectralFlux => "spectral_flux",
            AnalysisSource::BeatPhase => "beat_phase",
            AnalysisSource::Band => "band",
        }
    }

    #[must_use]
    pub fn from_canonical_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.canonical_name() == name)
    }
}

/// The curve applied to a normalized amount (`project.h:69-76`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum Interpolation {
    Step = 0,
    #[default]
    Linear = 1,
    Smoothstep = 2,
    EaseIn = 3,
    EaseOut = 4,
}

impl Interpolation {
    pub const ALL: [Interpolation; 5] = [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::Smoothstep,
        Interpolation::EaseIn,
        Interpolation::EaseOut,
    ];

    #[must_use]
    pub fn canonical_name(self) -> &'static str {
        match self {
            Interpolation::Step => "step",
            Interpolation::Linear => "linear",
            Interpolation::Smoothstep => "smoothstep",
            Interpolation::EaseIn => "ease_in",
            Interpolation::EaseOut => "ease_out",
        }
    }

    #[must_use]
    pub fn from_canonical_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|curve| curve.canonical_name() == name)
    }

    /// Shapes a normalized amount (`project.c:516-528`).
    ///
    /// Note `Step`: it is `< 1.0 ? 0.0 : 1.0`, so it only reaches the output
    /// maximum at exactly 1.0 — not a midpoint threshold.
    #[must_use]
    pub fn shape(self, amount: f64) -> f64 {
        match self {
            Interpolation::Step => {
                if amount < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
            Interpolation::Linear => amount,
            Interpolation::Smoothstep => amount * amount * (3.0 - 2.0 * amount),
            Interpolation::EaseIn => amount * amount,
            Interpolation::EaseOut => 1.0 - (1.0 - amount) * (1.0 - amount),
        }
    }
}

/// One parameter mapping (`project.h:211-221`).
///
/// The `parameter` field is the persisted settings key, e.g.
/// `"settings.loom.weight"`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterMapping {
    pub parameter: String,
    pub source: AnalysisSource,
    /// Must be zero unless `source` is [`AnalysisSource::Band`]
    /// (`scene_routes.h:41-47`).
    pub band_index: u16,
    pub input_min: f64,
    pub input_max: f64,
    pub output_min: f64,
    pub output_max: f64,
    pub interpolation: Interpolation,
    pub clamp: bool,
}

impl ParameterMapping {
    /// Maps a source sample to an output value (`musi_mapping_evaluate`,
    /// `project.c:542-566`).
    ///
    /// Returns `None` for a non-finite input, a degenerate input range
    /// (`input_max <= input_min`), or a non-finite result.
    ///
    /// The clamped and unclamped paths are deliberately *not* the same
    /// expression in C, and the difference is observable: with `clamp` the
    /// amount is clamped to 0..1 **and** `musi_interpolate` clamps again;
    /// without it, the amount is shaped unclamped, so a curve can extrapolate
    /// past the output endpoints. Reproduced as written.
    #[must_use]
    pub fn evaluate(&self, source_value: f64) -> Option<f64> {
        if !source_value.is_finite()
            || !self.input_min.is_finite()
            || !self.input_max.is_finite()
            || self.input_max <= self.input_min
            || !self.output_min.is_finite()
            || !self.output_max.is_finite()
        {
            return None;
        }
        let mut amount = (source_value - self.input_min) / (self.input_max - self.input_min);
        let result = if self.clamp {
            amount = amount.clamp(0.0, 1.0);
            let shaped = self.interpolation.shape(amount);
            self.output_min + (self.output_max - self.output_min) * shaped
        } else {
            let shaped = self.interpolation.shape(amount);
            self.output_min + (self.output_max - self.output_min) * shaped
        };
        result.is_finite().then_some(result)
    }

    /// The value this route produces for one sample, after the descriptor clamp
    /// and toggle quantization (`scene_route_output_value`,
    /// `scene_routes.c:303-327`).
    ///
    /// Toggle descriptors accept only their two canonical values, but routes are
    /// continuous, so the binary boundary is crossed at the descriptor's
    /// midpoint rather than rejecting almost every mapped frame.
    #[must_use]
    pub fn output_value(&self, descriptor: &SettingDescriptor, source_value: f64) -> Option<f64> {
        let mut value = self.evaluate(source_value)?;
        value = value.clamp(descriptor.minimum as f64, descriptor.maximum as f64);
        if descriptor.kind == SettingKind::Toggle {
            let midpoint = (descriptor.minimum as f64 + descriptor.maximum as f64) * 0.5;
            value = if value >= midpoint {
                descriptor.maximum as f64
            } else {
                descriptor.minimum as f64
            };
        }
        Some(value)
    }

    /// Well-formedness for a given scene (`scene_route_valid`,
    /// `scene_routes.h:41-48`).
    ///
    /// The flat-value rule is worth reading twice: a route whose output
    /// endpoints are equal is *rejected*, because `.musi` v1 cannot distinguish
    /// a full-range flat RMS route from a persisted slider constant. Flat values
    /// belong to the slider representation.
    #[must_use]
    pub fn is_valid_for(&self, scene: SceneId) -> bool {
        let Some((owner, _index, _descriptor)) = settings::descriptor_by_key(&self.parameter)
        else {
            return false;
        };
        if owner != scene {
            return false;
        }
        if self.source != AnalysisSource::Band && self.band_index != 0 {
            return false;
        }
        if self.source == AnalysisSource::Band
            && self.band_index as usize >= crate::audio::analyzer::MAX_BANDS
        {
            return false;
        }
        self.input_min.is_finite()
            && self.input_max.is_finite()
            && self.input_max > self.input_min
            && self.output_min.is_finite()
            && self.output_max.is_finite()
            && self.output_min != self.output_max
    }
}

/// The per-frame source values a route can bind to (`scene_routes.h:29-36`).
///
/// A mirror of [`super::SceneAudioFrame`]'s figures, deliberately kept free of
/// the scene module's drawing concerns so this stays headless.
#[derive(Clone, Copy, Debug, Default)]
pub struct RouteSources<'a> {
    pub bands: &'a [f32],
    pub rms: f32,
    pub peak: f32,
    pub spectral_flux: f32,
    pub beat_phase: f32,
}

impl<'a> RouteSources<'a> {
    #[must_use]
    pub fn from_audio(audio: &super::SceneAudioFrame<'a>) -> Self {
        Self {
            bands: audio.bands,
            rms: audio.rms,
            peak: audio.peak,
            spectral_flux: audio.spectral_flux,
            beat_phase: audio.beat_phase,
        }
    }

    /// Binds a source to its current value (`scene_routes_source_value`,
    /// `scene_routes.c:191-212`).
    ///
    /// `None` when the value is unavailable (an out-of-range band) or not
    /// finite. Callers skip the route for this frame rather than propagating a
    /// bad value.
    #[must_use]
    pub fn value(&self, source: AnalysisSource, band_index: u16) -> Option<f64> {
        let sample = match source {
            AnalysisSource::Rms => self.rms,
            AnalysisSource::Peak => self.peak,
            AnalysisSource::SpectralFlux => self.spectral_flux,
            AnalysisSource::BeatPhase => self.beat_phase,
            AnalysisSource::Band => *self.bands.get(band_index as usize)?,
        };
        sample.is_finite().then_some(sample as f64)
    }
}

/// Routes for one scene.
#[derive(Clone, Debug, Default)]
pub struct SceneRoutes {
    items: Vec<ParameterMapping>,
}

impl SceneRoutes {
    #[must_use]
    pub fn items(&self) -> &[ParameterMapping] {
        &self.items
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Every scene's routes (`scene_routes.h:23-25`).
#[derive(Clone, Debug, Default)]
pub struct RouteTable {
    scenes: [SceneRoutes; super::SCENE_COUNT],
}

/// Why a route could not be added (`scene_route_table_add`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("route is not valid for this scene")]
    Invalid,
    #[error("a route for this parameter already exists in the scene")]
    Duplicate,
    #[error("scene already holds the maximum of {ROUTES_PER_SCENE} routes")]
    Full,
}

impl RouteTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn scene(&self, scene: SceneId) -> &SceneRoutes {
        &self.scenes[scene.index()]
    }

    /// Adds a valid route, rejecting duplicates of the same parameter within the
    /// scene and additions beyond capacity (`scene_routes.h:50-53`).
    pub fn add(&mut self, scene: SceneId, route: ParameterMapping) -> Result<(), RouteError> {
        if !route.is_valid_for(scene) {
            return Err(RouteError::Invalid);
        }
        let routes = &mut self.scenes[scene.index()];
        if routes
            .items
            .iter()
            .any(|existing| existing.parameter == route.parameter)
        {
            return Err(RouteError::Duplicate);
        }
        if routes.items.len() >= ROUTES_PER_SCENE {
            return Err(RouteError::Full);
        }
        routes.items.push(route);
        Ok(())
    }

    /// Removes a route by index, returning whether it existed.
    pub fn remove(&mut self, scene: SceneId, index: usize) -> bool {
        let routes = &mut self.scenes[scene.index()];
        if index >= routes.items.len() {
            return false;
        }
        routes.items.remove(index);
        true
    }

    /// Copies `base` and applies this scene's routes on top
    /// (`scene_routes_apply`, `scene_routes.c:329-359`).
    ///
    /// A route whose source or evaluation fails this frame leaves the base value
    /// untouched. Deterministic for identical inputs, which is what keeps a
    /// routed preview and a routed export identical.
    ///
    /// Returns `None` if `base` is not valid, matching C's refusal to route on
    /// top of invalid settings.
    #[must_use]
    pub fn apply(
        &self,
        scene: SceneId,
        sources: &RouteSources<'_>,
        base: &SceneSettings,
    ) -> Option<SceneSettings> {
        if !base.is_valid() {
            return None;
        }
        let mut staged = *base;
        for route in &self.scenes[scene.index()].items {
            let Some((owner, index, descriptor)) = settings::descriptor_by_key(&route.parameter)
            else {
                continue;
            };
            if owner != scene {
                continue;
            }
            let Some(source_value) = sources.value(route.source, route.band_index) else {
                continue;
            };
            let Some(mapped) = route.output_value(descriptor, source_value) else {
                continue;
            };
            // A mapped value that the descriptor still rejects is a bug in
            // output_value, not something to paper over. C returns failure here
            // rather than continuing, and so does this.
            if !staged.set(scene, index, mapped as f32) {
                return None;
            }
        }
        Some(staged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::index;

    fn band_route(parameter: &str) -> ParameterMapping {
        ParameterMapping {
            parameter: parameter.to_string(),
            source: AnalysisSource::Band,
            band_index: 2,
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.4,
            output_max: 2.2,
            interpolation: Interpolation::Linear,
            clamp: true,
        }
    }

    #[test]
    fn enum_discriminants_and_names_match_the_codec() {
        assert_eq!(AnalysisSource::Rms as u32, 0);
        assert_eq!(AnalysisSource::Band as u32, 4);
        assert_eq!(Interpolation::Step as u32, 0);
        assert_eq!(Interpolation::EaseOut as u32, 4);
        assert_eq!(Interpolation::default(), Interpolation::Linear);
        for source in AnalysisSource::ALL {
            assert_eq!(
                AnalysisSource::from_canonical_name(source.canonical_name()),
                Some(source)
            );
        }
        for curve in Interpolation::ALL {
            assert_eq!(
                Interpolation::from_canonical_name(curve.canonical_name()),
                Some(curve)
            );
        }
    }

    #[test]
    fn step_only_reaches_the_maximum_at_exactly_one() {
        assert_eq!(Interpolation::Step.shape(0.0), 0.0);
        assert_eq!(Interpolation::Step.shape(0.999), 0.0);
        assert_eq!(Interpolation::Step.shape(1.0), 1.0);
    }

    #[test]
    fn curves_are_the_c_formulas() {
        assert_eq!(Interpolation::Linear.shape(0.25), 0.25);
        assert_eq!(Interpolation::Smoothstep.shape(0.5), 0.5);
        assert_eq!(Interpolation::EaseIn.shape(0.5), 0.25);
        assert_eq!(Interpolation::EaseOut.shape(0.5), 0.75);
    }

    #[test]
    fn evaluation_rejects_a_degenerate_input_range() {
        let mut route = band_route("settings.loom.weight");
        route.input_max = route.input_min;
        assert_eq!(route.evaluate(0.5), None);
        route.input_max = f64::NAN;
        assert_eq!(route.evaluate(0.5), None);
        let good = band_route("settings.loom.weight");
        assert_eq!(good.evaluate(f64::NAN), None);
    }

    #[test]
    fn clamping_bounds_the_output_and_noclamp_extrapolates() {
        let clamped = band_route("settings.loom.weight");
        assert_eq!(clamped.evaluate(-5.0), Some(0.4), "clamped to output_min");
        assert_eq!(clamped.evaluate(5.0), Some(2.2), "clamped to output_max");

        let unclamped = ParameterMapping {
            clamp: false,
            ..band_route("settings.loom.weight")
        };
        let low = unclamped.evaluate(-1.0).unwrap();
        assert!(
            low < 0.4,
            "an unclamped linear route extrapolates below output_min, got {low}"
        );
    }

    #[test]
    fn output_value_clamps_to_the_descriptor_range() {
        // Loom weight is 0.40..2.50, so a route reaching 2.2 stays inside it.
        let descriptor = settings::descriptor(SceneId::Loom, index::loom::WEIGHT).unwrap();
        let route = band_route("settings.loom.weight");
        assert_eq!(route.output_value(descriptor, 1.0), Some(2.2));

        // A route aiming past the descriptor maximum is clamped, not rejected.
        let wide = ParameterMapping {
            output_max: 99.0,
            ..route
        };
        assert_eq!(wide.output_value(descriptor, 1.0), Some(2.50));
    }

    #[test]
    fn a_toggle_crosses_at_the_descriptor_midpoint() {
        let descriptor = settings::descriptor(SceneId::SongAtlas, index::atlas::WIREFRAME).unwrap();
        assert_eq!(descriptor.kind, SettingKind::Toggle);
        let route = ParameterMapping {
            parameter: "settings.atlas.wireframe".to_string(),
            source: AnalysisSource::Rms,
            band_index: 0,
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
            interpolation: Interpolation::Linear,
            clamp: true,
        };
        assert_eq!(route.output_value(descriptor, 0.49), Some(0.0));
        assert_eq!(route.output_value(descriptor, 0.5), Some(1.0));
    }

    #[test]
    fn a_flat_route_is_rejected_because_v1_cannot_tell_it_from_a_slider() {
        let flat = ParameterMapping {
            output_max: 0.4,
            ..band_route("settings.loom.weight")
        };
        assert_eq!(flat.output_min, flat.output_max);
        assert!(!flat.is_valid_for(SceneId::Loom));
    }

    #[test]
    fn a_band_index_is_only_allowed_for_the_band_source() {
        let mut route = band_route("settings.loom.weight");
        route.source = AnalysisSource::Rms;
        assert!(
            !route.is_valid_for(SceneId::Loom),
            "band_index must be zero for non-band sources"
        );
        route.band_index = 0;
        assert!(route.is_valid_for(SceneId::Loom));
    }

    #[test]
    fn a_route_must_target_its_own_scene() {
        let route = band_route("settings.loom.weight");
        assert!(route.is_valid_for(SceneId::Loom));
        assert!(!route.is_valid_for(SceneId::Cadence));
        let unknown = band_route("settings.nope.nope");
        assert!(!unknown.is_valid_for(SceneId::Loom));
    }

    #[test]
    fn the_table_rejects_duplicates_and_respects_capacity() {
        let mut table = RouteTable::new();
        table
            .add(SceneId::Loom, band_route("settings.loom.weight"))
            .unwrap();
        assert_eq!(
            table
                .add(SceneId::Loom, band_route("settings.loom.weight"))
                .unwrap_err(),
            RouteError::Duplicate
        );
        assert_eq!(
            table
                .add(SceneId::Loom, band_route("settings.cadence.glow"))
                .unwrap_err(),
            RouteError::Invalid
        );
        assert_eq!(table.scene(SceneId::Loom).len(), 1);
        assert!(table.remove(SceneId::Loom, 0));
        assert!(!table.remove(SceneId::Loom, 0));
        assert!(table.scene(SceneId::Loom).is_empty());
    }

    #[test]
    fn applying_a_route_overrides_only_its_own_parameter() {
        let mut table = RouteTable::new();
        table
            .add(SceneId::Loom, band_route("settings.loom.weight"))
            .unwrap();
        let bands = [0.0f32, 0.0, 1.0, 0.0];
        let sources = RouteSources {
            bands: &bands,
            ..Default::default()
        };
        let base = SceneSettings::default();
        let effective = table.apply(SceneId::Loom, &sources, &base).unwrap();
        assert_eq!(effective.get(SceneId::Loom, index::loom::WEIGHT), 2.2);
        // Untouched settings keep their base values.
        assert_eq!(
            effective.get(SceneId::Loom, index::loom::DENSITY),
            base.get(SceneId::Loom, index::loom::DENSITY)
        );
    }

    #[test]
    fn a_missing_band_leaves_the_base_value_alone() {
        let mut table = RouteTable::new();
        table
            .add(SceneId::Loom, band_route("settings.loom.weight"))
            .unwrap();
        // Band 2 does not exist in a two-band spectrum.
        let bands = [0.5f32, 0.5];
        let sources = RouteSources {
            bands: &bands,
            ..Default::default()
        };
        let base = SceneSettings::default();
        let effective = table.apply(SceneId::Loom, &sources, &base).unwrap();
        assert_eq!(
            effective.get(SceneId::Loom, index::loom::WEIGHT),
            base.get(SceneId::Loom, index::loom::WEIGHT),
            "an unavailable source must not propagate a bad value"
        );
    }

    #[test]
    fn applying_is_deterministic_which_is_what_keeps_preview_and_export_equal() {
        let mut table = RouteTable::new();
        table
            .add(SceneId::Loom, band_route("settings.loom.weight"))
            .unwrap();
        let bands = [0.1f32, 0.2, 0.375, 0.4];
        let sources = RouteSources {
            bands: &bands,
            rms: 0.3,
            ..Default::default()
        };
        let base = SceneSettings::default();
        let first = table.apply(SceneId::Loom, &sources, &base).unwrap();
        let second = table.apply(SceneId::Loom, &sources, &base).unwrap();
        assert_eq!(first, second);
    }
}
