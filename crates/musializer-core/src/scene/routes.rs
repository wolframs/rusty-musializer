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

/// Which per-frame figure drives a route (`project.h:60-67`, plus `Time`).
///
/// Discriminants are persisted in `.musi`, so they are a compatibility surface.
/// `Time` post-dates the frozen C (UX0-C15, 2026-08-04): the same eight-second
/// triangle clock the caption effects' Time drive uses, so a route can breathe
/// without any audio at all. Additive — every C-era token keeps its meaning,
/// which is what keeps the route differential harnesses green.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AnalysisSource {
    Rms = 0,
    Peak = 1,
    SpectralFlux = 2,
    BeatPhase = 3,
    /// One analyzer band, selected by `band_index`.
    Band = 4,
    /// The deterministic eight-second triangle clock ([`time_triangle`]).
    Time = 5,
}

impl AnalysisSource {
    pub const ALL: [AnalysisSource; 6] = [
        AnalysisSource::Rms,
        AnalysisSource::Peak,
        AnalysisSource::SpectralFlux,
        AnalysisSource::BeatPhase,
        AnalysisSource::Band,
        AnalysisSource::Time,
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
            AnalysisSource::Time => "time",
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
        Self::evaluate_mapping(
            source_value,
            self.input_min,
            self.input_max,
            self.output_min,
            self.output_max,
            self.interpolation,
            self.clamp,
        )
    }

    /// The mapping arithmetic itself, factored out of [`ParameterMapping`] so the
    /// caption effects' drive tuning (`project::caption_effects`) applies the
    /// *same* semantics — including the C's clamp asymmetry — instead of a
    /// re-derivation that could drift. The body is byte-for-byte the pinned
    /// `musi_mapping_evaluate` port; the route differential harness is the
    /// contract on it.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn evaluate_mapping(
        source_value: f64,
        input_min: f64,
        input_max: f64,
        output_min: f64,
        output_max: f64,
        interpolation: Interpolation,
        clamp: bool,
    ) -> Option<f64> {
        if !source_value.is_finite()
            || !input_min.is_finite()
            || !input_max.is_finite()
            || input_max <= input_min
            || !output_min.is_finite()
            || !output_max.is_finite()
        {
            return None;
        }
        let mut amount = (source_value - input_min) / (input_max - input_min);
        let result = if clamp {
            amount = amount.clamp(0.0, 1.0);
            let shaped = interpolation.shape(amount);
            output_min + (output_max - output_min) * shaped
        } else {
            let shaped = interpolation.shape(amount);
            output_min + (output_max - output_min) * shaped
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

/// The canonical eight-second triangle clock shared by [`AnalysisSource::Time`]
/// and the caption effects' Time drive: 0 at each cycle boundary, 1 at the
/// four-second midpoint, continuous across the wrap.
///
/// One definition on purpose — a scene route and a caption glow driven by
/// "Time" must peak on the same frame or the two features stop looking like
/// one clock.
#[must_use]
pub fn time_triangle(time_seconds: f64) -> f64 {
    let cycle = (time_seconds / 8.0).fract();
    1.0 - (2.0 * cycle - 1.0).abs()
}

/// The per-frame source values a route can bind to (`scene_routes.h:29-36`).
///
/// A mirror of [`super::SceneAudioFrame`]'s figures plus the deterministic
/// clock, deliberately kept free of the scene module's drawing concerns so
/// this stays headless.
#[derive(Clone, Copy, Debug, Default)]
pub struct RouteSources<'a> {
    pub bands: &'a [f32],
    pub rms: f32,
    pub peak: f32,
    pub spectral_flux: f32,
    pub beat_phase: f32,
    /// Playback (or export frame) time. `SceneAudioFrame` carries no clock, so
    /// [`RouteSources::from_audio`] takes it separately.
    pub time_seconds: f64,
}

impl<'a> RouteSources<'a> {
    #[must_use]
    pub fn from_audio(audio: &super::SceneAudioFrame<'a>, time_seconds: f64) -> Self {
        Self {
            bands: audio.bands,
            rms: audio.rms,
            peak: audio.peak,
            spectral_flux: audio.spectral_flux,
            beat_phase: audio.beat_phase,
            time_seconds,
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
            AnalysisSource::Time => {
                let wave = time_triangle(self.time_seconds);
                return wave.is_finite().then_some(wave);
            }
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
        // Capacity before duplicates, matching `scene_route_table_add`'s
        // precedence (`scene_routes.c:65-78`). C returns a bare `false` either
        // way so the order is unobservable there, but these errors are richer
        // than a boolean and keeping the precedence identical means mapping them
        // back to one can never disagree with the oracle.
        if routes.items.len() >= ROUTES_PER_SCENE {
            return Err(RouteError::Full);
        }
        if routes
            .items
            .iter()
            .any(|existing| existing.parameter == route.parameter)
        {
            return Err(RouteError::Duplicate);
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

// -- Persistence: constants, routes, and the `--route` grammar ----------------
//
// Moved here from `project::model` (Agent G, session 3), where they lived because
// they need the `.musi` schema's bounds. They belong beside the evaluation they
// persist, and they had to move **wholesale**: the constant rule and the route
// rule are two halves of one decision. `.musi` v1 spells a slider value as a
// full-range RMS mapping with equal output endpoints, which is exactly why
// `ParameterMapping::is_valid_for` rejects a *route* with equal endpoints - each
// parameter is persisted as one or the other and never both. Splitting the two
// rules across modules is what would let one parameter acquire both spellings, an
// ambiguity the format cannot represent.
//
// The two `.musi` schema bounds these read stay in `project::model`, because the
// project validator reads them too and they are format limits rather than route
// semantics.

/// `scene_settings_mapping_supported` (`scene_settings.c:407-417`): the canonical
/// spelling of a persisted **slider constant**.
///
/// A constant is a full-range RMS mapping whose output endpoints are equal. That
/// is why a *route* with equal endpoints is rejected
/// ([`ParameterMapping::is_valid_for`]): v1 cannot distinguish the two, so each
/// parameter is persisted as exactly one of them.
#[must_use]
pub fn mapping_is_constant(mapping: &ParameterMapping) -> bool {
    settings::descriptor_by_key(&mapping.parameter).is_some()
        && mapping.source == AnalysisSource::Rms
        && mapping.band_index == 0
        && mapping.input_min == 0.0
        && mapping.input_max == 1.0
        && mapping.output_min == mapping.output_max
        && mapping.output_min.is_finite()
        && mapping.interpolation == Interpolation::Linear
        && mapping.clamp
}

/// The canonical constant mapping for one setting value
/// (`scene_settings_export_mappings`, `scene_settings.c:434-467`).
#[must_use]
pub fn constant_mapping(key: &str, value: f32) -> ParameterMapping {
    ParameterMapping {
        parameter: key.to_owned(),
        source: AnalysisSource::Rms,
        band_index: 0,
        input_min: 0.0,
        input_max: 1.0,
        output_min: f64::from(value),
        output_max: f64::from(value),
        interpolation: Interpolation::Linear,
        clamp: true,
    }
}

/// `scene_routes_mappings_supported` (`scene_routes.c:224-237`): every mapping is
/// either a canonical constant or a valid dynamic route for its parameter's scene,
/// with parameter names unique across the list.
#[must_use]
pub fn mappings_supported(mappings: &[ParameterMapping]) -> bool {
    if mappings.len() > crate::project::model::MAX_MAPPINGS_PER_SCENE {
        return false;
    }
    for (index, mapping) in mappings.iter().enumerate() {
        let supported = mapping_is_constant(mapping)
            || settings::descriptor_by_key(&mapping.parameter)
                .is_some_and(|(scene, _, _)| mapping.is_valid_for(scene));
        if !supported {
            return false;
        }
        if mappings[..index]
            .iter()
            .any(|other| other.parameter == mapping.parameter)
        {
            return false;
        }
    }
    true
}

/// `scene_routes_export_mappings` (`scene_routes.c:239-269`): every setting as its
/// canonical constant, except settings driven by a route, which persist the route
/// in that position instead.
///
/// Deterministic order — scene by scene, control by control — so saves are
/// byte-stable. A route with no matching constant slot means the route table and
/// the settings tables disagree, and that refuses to save rather than dropping the
/// route.
pub fn export_mappings(
    settings_values: &SceneSettings,
    routes: Option<&RouteTable>,
) -> Option<Vec<ParameterMapping>> {
    if !settings_values.is_valid() {
        return None;
    }
    let mut mappings = Vec::with_capacity(crate::project::model::MAX_MAPPINGS_PER_SCENE);
    for scene in SceneId::ALL {
        for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
            mappings.push(constant_mapping(
                descriptor.key,
                settings_values.get(scene, index),
            ));
        }
    }
    if let Some(routes) = routes {
        for scene in SceneId::ALL {
            for route in routes.scene(scene).items() {
                let slot = mappings
                    .iter_mut()
                    .find(|mapping| mapping.parameter == route.parameter)?;
                *slot = route.clone();
            }
        }
    }
    Some(mappings)
}

/// `scene_routes_import_mappings` (`scene_routes.c:271-301`): partitions project
/// mappings into slider constants and dynamic routes.
///
/// All-or-nothing: a mapping that is neither refuses the whole import, so nothing
/// is silently dropped. Settings for routed parameters keep their scene defaults.
pub fn import_mappings(mappings: &[ParameterMapping]) -> Option<(SceneSettings, RouteTable)> {
    if !mappings_supported(mappings) {
        return None;
    }
    let mut staged_settings = SceneSettings::new();
    let mut staged_routes = RouteTable::new();
    for mapping in mappings {
        let (scene, index, _) = settings::descriptor_by_key(&mapping.parameter)?;
        if mapping_is_constant(mapping) {
            if !staged_settings.set(scene, index, mapping.output_min as f32) {
                return None;
            }
        } else {
            staged_routes.add(scene, mapping.clone()).ok()?;
        }
    }
    Some((staged_settings, staged_routes))
}

/// Longest `--route` spec the parser will look at (`scene_routes.c:97`).
pub const ROUTE_SPEC_MAX_BYTES: usize = 255;

/// `scene_route_parse_spec` (`scene_routes.c:109-189`).
///
/// Grammar:
/// `parameter:source:band:in_min:in_max:out_min:out_max[:curve][:clamp|noclamp]`,
/// e.g. `loom.weight:band:2:0:1:0.4:2.2:smoothstep`. The `settings.` key prefix
/// may be omitted; source and curve names are this codec's canonical names; the
/// curve defaults to linear and clamping is on unless `noclamp` is given.
pub fn parse_route_spec(spec: &str) -> Option<(SceneId, ParameterMapping)> {
    if spec.len() > ROUTE_SPEC_MAX_BYTES {
        return None;
    }
    let fields: Vec<&str> = spec.split(':').collect();
    if fields.len() < 7 || fields.len() > 9 {
        return None;
    }
    // The persisted keys all carry the "settings." prefix; accept the short form
    // people actually type.
    let parameter = if fields[0].starts_with("settings.") {
        fields[0].to_owned()
    } else {
        format!("settings.{}", fields[0])
    };
    if parameter.len() > crate::project::model::capacity::PARAMETER {
        return None;
    }
    let parse_double = |text: &str| -> Option<f64> {
        let value: f64 = text.parse().ok()?;
        value.is_finite().then_some(value)
    };
    let mut route = ParameterMapping {
        parameter,
        source: AnalysisSource::from_canonical_name(fields[1])?,
        band_index: fields[2].parse().ok()?,
        input_min: parse_double(fields[3])?,
        input_max: parse_double(fields[4])?,
        output_min: parse_double(fields[5])?,
        output_max: parse_double(fields[6])?,
        interpolation: Interpolation::Linear,
        clamp: true,
    };
    for extra in &fields[7..] {
        match *extra {
            "clamp" => route.clamp = true,
            "noclamp" => route.clamp = false,
            name => route.interpolation = Interpolation::from_canonical_name(name)?,
        }
    }
    let (scene, _, _) = settings::descriptor_by_key(&route.parameter)?;
    route.is_valid_for(scene).then_some((scene, route))
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

    // -- route persistence ---------------------------------------------------
    //
    // Moved from `project::model`'s suite with the functions they cover.

    /// The canonical constant spelling of one setting value, which is what a
    /// mapping that is *not* a route has to look like.
    fn constant(parameter: &str) -> ParameterMapping {
        constant_mapping(parameter, 0.5)
    }

    #[test]
    fn exported_mappings_are_one_constant_per_control() {
        let settings_values = SceneSettings::new();
        let mappings = export_mappings(&settings_values, None).unwrap();
        let expected: usize = SceneId::ALL.into_iter().map(settings::count).sum();
        assert_eq!(mappings.len(), expected);
        assert!(mappings.iter().all(mapping_is_constant));
        assert!(mappings_supported(&mappings));
    }

    #[test]
    fn a_route_replaces_its_constant_in_place() {
        let settings_values = SceneSettings::new();
        let mut routes = RouteTable::new();
        let (scene, route) =
            parse_route_spec("spectrum.amplitude:band:2:0:1:0.4:2.2:smoothstep").unwrap();
        assert_eq!(scene, SceneId::Spectrum);
        routes.add(scene, route.clone()).unwrap();

        let baseline = export_mappings(&settings_values, None).unwrap();
        let mappings = export_mappings(&settings_values, Some(&routes)).unwrap();
        assert_eq!(
            mappings.len(),
            baseline.len(),
            "no slot is added or removed"
        );
        let at = mappings
            .iter()
            .position(|mapping| mapping.parameter == "settings.spectrum.amplitude")
            .unwrap();
        assert_eq!(mappings[at], route);
        assert!(mappings_supported(&mappings));

        let (imported_settings, imported_routes) = import_mappings(&mappings).unwrap();
        assert_eq!(imported_routes.scene(SceneId::Spectrum).items(), [route]);
        // A routed parameter keeps its scene default in the slider table.
        assert_eq!(
            imported_settings.get(SceneId::Spectrum, 0),
            settings::descriptor(SceneId::Spectrum, 0)
                .unwrap()
                .default_value
        );
    }

    #[test]
    fn import_is_all_or_nothing() {
        let mut mappings = export_mappings(&SceneSettings::new(), None).unwrap();
        mappings.push(constant("settings.not.a.control"));
        assert!(!mappings_supported(&mappings));
        assert!(import_mappings(&mappings).is_none());
    }

    #[test]
    fn a_flat_route_is_not_a_route() {
        // v1 cannot distinguish a full-range flat RMS route from a persisted
        // slider constant, so flat values belong to the slider representation.
        let (_, route) = parse_route_spec("spectrum.amplitude:rms:0:0:1:0.5:0.5").unzip();
        assert!(route.is_none());
    }

    #[test]
    fn route_specs_are_parsed_with_defaults_and_rejected_when_malformed() {
        let (scene, route) = parse_route_spec("settings.loom.weight:rms:0:0:1:0.4:2.2").unwrap();
        assert_eq!(scene, SceneId::Loom);
        assert_eq!(route.interpolation, Interpolation::Linear);
        assert!(route.clamp, "clamping is on unless noclamp is given");

        let (_, route) = parse_route_spec("loom.weight:rms:0:0:1:0.4:2.2:ease_in:noclamp").unwrap();
        assert_eq!(route.interpolation, Interpolation::EaseIn);
        assert!(!route.clamp);

        for spec in [
            "loom.weight:rms:0:0:1:0.4",           // too few fields
            "loom.weight:rms:0:0:1:0.4:2.2:a:b:c", // too many
            "loom.weight:bogus:0:0:1:0.4:2.2",     // unknown source
            "loom.weight:rms:0:0:1:0.4:2.2:bogus", // unknown curve
            "loom.weight:rms:0:1:0:0.4:2.2",       // input_max <= input_min
            "loom.weight:rms:3:0:1:0.4:2.2",       // band index without band
            "nope.nothing:rms:0:0:1:0.4:2.2",      // unknown parameter
            "loom.weight:rms:0:nan:1:0.4:2.2",     // non-finite
            "loom.weight:rms:70000:0:1:0.4:2.2",   // band index out of u16
        ] {
            assert!(parse_route_spec(spec).is_none(), "accepted {spec}");
        }
    }

    #[test]
    fn the_time_triangle_is_continuous_and_bounded() {
        assert!((time_triangle(0.0)).abs() < 1e-12);
        assert!((time_triangle(4.0) - 1.0).abs() < 1e-12);
        assert!((time_triangle(8.0)).abs() < 1e-12);
        assert!((time_triangle(7.999) - time_triangle(8.001)).abs() < 1e-3);
        for step in 0..800 {
            let value = time_triangle(step as f64 * 0.037);
            assert!((0.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn a_time_route_reads_the_clock_and_needs_no_audio() {
        let sources = RouteSources {
            time_seconds: 4.0,
            ..RouteSources::default()
        };
        assert_eq!(sources.value(AnalysisSource::Time, 0), Some(1.0));

        let (scene, route) = parse_route_spec("loom.weight:time:0:0:1:0.4:2.2").unwrap();
        assert_eq!(route.source, AnalysisSource::Time);
        assert!(route.is_valid_for(scene));
        // The band rule covers Time exactly as it covers RMS: a nonzero band
        // index refuses the route.
        assert!(parse_route_spec("loom.weight:time:3:0:1:0.4:2.2").is_none());
    }
}
