//! The scene contract: identifiers, the per-frame input every scene reads, and
//! the registry that binds deterministic state to it.
//!
//! **This module is a shared contract.** It is owned by the integration owner
//! and consumed by Agents C, D and F. A scene agent that needs a new field asks
//! for it rather than adding it, because the other scene agent is compiling
//! against the same file. See REWRITE_PLAN.md, "Shared contracts land first".
//!
//! Ported from `../musializer/src/scene.h` and `scene.c`. The split from C: C's
//! `Scene_Descriptor` carries `init`/`update`/`draw`/`unload` function pointers
//! over a `void *state`. Here the deterministic half (`update`) is a trait in
//! this crate, and drawing lives in `musializer-app` where raylib is allowed.
//! That is the split the plan asks for, and it is what lets scene state be
//! tested headlessly.

pub mod events;
pub mod routes;
pub mod settings;

pub use settings::{SceneSettings, SettingDescriptor, SettingKind, SettingsSnapshot};

use std::any::Any;

/// The ten scenes, in registry order (`../musializer/src/scene.h:17-29`).
///
/// The discriminants are the C enum's values and are load-bearing: `.musi`
/// projects and the `--scene` CLI flag both resolve through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum SceneId {
    Spectrum = 0,
    PulseField = 1,
    OrbitalLattice = 2,
    AsciiField = 3,
    SongAtlas = 4,
    SpectralTerrarium = 5,
    Constellation = 6,
    Cadence = 7,
    Loom = 8,
    Pentagram = 9,
}

/// Number of scenes. C's `COUNT_SCENES` and `SCENE_SETTINGS_SCENE_COUNT`
/// (`scene.h:28`, `scene_settings_values.h:8`) — the two must stay equal.
pub const SCENE_COUNT: usize = 10;

impl SceneId {
    /// Every scene in registry order.
    pub const ALL: [SceneId; SCENE_COUNT] = [
        SceneId::Spectrum,
        SceneId::PulseField,
        SceneId::OrbitalLattice,
        SceneId::AsciiField,
        SceneId::SongAtlas,
        SceneId::SpectralTerrarium,
        SceneId::Constellation,
        SceneId::Cadence,
        SceneId::Loom,
        SceneId::Pentagram,
    ];

    /// Registry index, equal to the C enum value.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The display name the UI shows (`scene_*.c` descriptor `.name` fields).
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            SceneId::Spectrum => "Spectrum",
            SceneId::PulseField => "Pulse Field",
            SceneId::OrbitalLattice => "Orbital Lattice",
            SceneId::AsciiField => "ASCII Field",
            SceneId::SongAtlas => "Song Atlas",
            SceneId::SpectralTerrarium => "Spectral Terrarium",
            SceneId::Constellation => "Constellation",
            SceneId::Cadence => "Cadence",
            SceneId::Loom => "Loom",
            SceneId::Pentagram => "Pentagram Orbits",
        }
    }

    /// The persisted identifier. This is what `.musi` files and `--scene`
    /// contain, so it is a compatibility surface — never rename one
    /// (`../musializer/src/scene.c:47-63`).
    #[must_use]
    pub fn stable_name(self) -> &'static str {
        match self {
            SceneId::Spectrum => "spectrum",
            SceneId::PulseField => "pulse",
            SceneId::OrbitalLattice => "orbital",
            SceneId::AsciiField => "ascii",
            SceneId::SongAtlas => "atlas",
            SceneId::SpectralTerrarium => "terrarium",
            SceneId::Constellation => "constellation",
            SceneId::Cadence => "cadence",
            SceneId::Loom => "loom",
            SceneId::Pentagram => "pentagram",
        }
    }

    /// Resolves a persisted/CLI name.
    ///
    /// C falls back to `spectrum` for an unknown id in `scene_stable_name`, but
    /// that is the id→name direction. This is name→id, where an unknown name is
    /// a caller error worth surfacing, so it returns `None`.
    #[must_use]
    pub fn from_stable_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.stable_name() == name)
    }
}

/// The measured audio evidence lane for one frame (`scene.h:31-40`).
///
/// `bands` and `trails` borrow the analyzer's smoothed/smeared band arrays;
/// they are always the same length.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneAudioFrame<'a> {
    /// Smoothed normalized band amplitudes — what a bar height is drawn from.
    pub bands: &'a [f32],
    /// The slower trailing smear behind `bands`.
    pub trails: &'a [f32],
    pub rms: f32,
    pub peak: f32,
    pub spectral_flux: f32,
    pub beat_phase: f32,
    pub onset: bool,
}

impl<'a> SceneAudioFrame<'a> {
    #[must_use]
    pub fn bands_count(&self) -> usize {
        self.bands.len()
    }

    /// Derives the aggregate audio figures from a spectrum view, exactly as
    /// `make_scene_frame` does (`../musializer/src/plug.c:1116-1128`).
    ///
    /// Note what flux is here: the summed *positive* excursion of each band
    /// above its own trail, averaged over bands. It is not a frame-to-frame
    /// difference.
    #[must_use]
    pub fn from_spectrum(smooth: &'a [f32], smear: &'a [f32]) -> Self {
        debug_assert_eq!(smooth.len(), smear.len());
        let mut square_sum = 0.0f32;
        let mut peak = 0.0f32;
        let mut flux = 0.0f32;
        for (i, &band) in smooth.iter().enumerate() {
            square_sum += band * band;
            if peak < band {
                peak = band;
            }
            if smear[i] < band {
                flux += band - smear[i];
            }
        }
        let count = smooth.len();
        let rms = if count > 0 {
            (square_sum / count as f32).sqrt()
        } else {
            0.0
        };
        let flux = if count > 0 { flux / count as f32 } else { 0.0 };
        Self {
            bands: smooth,
            trails: smear,
            rms,
            peak,
            spectral_flux: flux,
            beat_phase: 0.0,
            // `onset` is `flux > 0.08` (plug.c:1139). Kept here so the threshold
            // lives in one place rather than in every caller.
            onset: flux > ONSET_FLUX_THRESHOLD,
        }
    }
}

/// The onset threshold on spectral flux (`../musializer/src/plug.c:1139`).
pub const ONSET_FLUX_THRESHOLD: f32 = 0.08;

/// The model-derived interpretive lane, held until the next cue begins
/// (`../musializer/src/semantic_lane.h:9-16`).
///
/// Deliberately separate from [`SceneAudioFrame`]: measured audio, lyric timing,
/// model interpretation, and manual events are four evidence lanes and the
/// rewrite keeps them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SemanticFrame {
    pub available: bool,
    pub source_id: u64,
    pub energy: f32,
    pub tension: f32,
    pub valence: f32,
    pub confidence: f32,
}

/// One timed lyric cue (`../musializer/src/lyrics.h:17-22`).
///
/// C uses a fixed 512-byte text buffer because the struct lives in realtime
/// state; Rust can own a `String` without that constraint. Agent B owns the full
/// lyrics model and may extend this — the contract here is only what a scene
/// reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricCue {
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

/// Everything a scene sees for one frame (`scene.h:42-52`).
///
/// The lifetimes are the contract: a scene borrows its inputs for the duration
/// of the frame and may not retain them. Anything a scene needs to remember
/// across frames belongs in its own state.
#[derive(Clone, Copy, Debug)]
pub struct SceneFrame<'a> {
    pub time_seconds: f64,
    pub duration_seconds: f64,
    /// The deterministic scene-clock delta, not wall clock. Preview and export
    /// both come through here, which is what makes them agree.
    pub delta_seconds: f32,
    pub frame_index: u64,
    pub audio: SceneAudioFrame<'a>,
    pub semantic: SemanticFrame,
    pub lyric: Option<&'a LyricCue>,
    pub events: events::EventTimelineView<'a>,
    /// The *effective* settings: the editable values overlaid with any routed
    /// per-frame snapshot. Preview and export read the same value here, which is
    /// what keeps routed parameters identical between them
    /// (`../musializer/src/plug.c:1147-1166`).
    pub settings: &'a SceneSettings,
}

impl<'a> SceneFrame<'a> {
    /// A frame with no audio, no semantics, and default settings — the shape an
    /// idle preview or a headless state test starts from.
    #[must_use]
    pub fn idle(settings: &'a SceneSettings) -> Self {
        Self {
            time_seconds: 0.0,
            duration_seconds: 0.0,
            delta_seconds: 0.0,
            frame_index: 0,
            audio: SceneAudioFrame::default(),
            semantic: SemanticFrame::default(),
            lyric: None,
            events: events::EventTimelineView::EMPTY,
            settings,
        }
    }

    /// Reads one setting for a scene, clamped and defaulted as C does.
    #[must_use]
    pub fn setting(&self, scene: SceneId, index: usize) -> f32 {
        self.settings.get(scene, index)
    }
}

/// The deterministic half of a scene: its state and its per-frame update.
///
/// Drawing is deliberately absent. It needs raylib, so it lives in
/// `musializer-app`; keeping `update` here is what makes scene state testable
/// without a window, which is the single practice that made the C suite grow
/// from 175 to 327 tests.
///
/// Determinism is a hard requirement: for the same seed, settings, and frame
/// sequence, `update` must produce the same state. No wall clock, no RNG that
/// is not seeded from `seed`, no I/O.
pub trait SceneState: Any + Send {
    fn id(&self) -> SceneId;

    /// Advances deterministic state. A scene with no state leaves this empty —
    /// Spectrum is one (`../musializer/src/scene_spectrum.c:149-154` sets no
    /// `update`), and that is fine.
    fn update(&mut self, frame: &SceneFrame<'_>) {
        let _ = frame;
    }

    /// Downcast hook so the drawing half in `musializer-app` can recover the
    /// concrete state type. Implementations are always `fn as_any(&self) -> &dyn
    /// Any { self }`.
    fn as_any(&self) -> &dyn Any;
}

/// Registry entry for one scene (`scene.h:77-86`).
#[derive(Clone, Copy)]
pub struct SceneDescriptor {
    pub id: SceneId,
    /// Bumped when the state layout changes, so a rebind can discard stale
    /// state instead of reinterpreting it.
    pub state_version: u32,
    pub make_state: fn(seed: u64) -> Box<dyn SceneState>,
}

impl SceneDescriptor {
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.id.display_name()
    }

    #[must_use]
    pub fn stable_name(&self) -> &'static str {
        self.id.stable_name()
    }
}

/// A scene with no deterministic state.
///
/// Spectrum is entirely a function of the frame, so it needs no state at all.
/// Rather than special-casing `Option<Box<dyn SceneState>>` everywhere, a
/// stateless scene gets one of these.
#[derive(Debug)]
pub struct StatelessScene(SceneId);

impl StatelessScene {
    #[must_use]
    pub fn new(id: SceneId) -> Self {
        Self(id)
    }
}

impl SceneState for StatelessScene {
    fn id(&self) -> SceneId {
        self.0
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A bound scene: its descriptor, its seed, and its live state (`scene.h:69-75`).
pub struct SceneInstance {
    descriptor: SceneDescriptor,
    seed: u64,
    state: Box<dyn SceneState>,
}

impl SceneInstance {
    #[must_use]
    pub fn new(descriptor: SceneDescriptor, seed: u64) -> Self {
        let state = (descriptor.make_state)(seed);
        debug_assert_eq!(
            state.id(),
            descriptor.id,
            "descriptor and state disagree on id"
        );
        Self {
            descriptor,
            seed,
            state,
        }
    }

    #[must_use]
    pub fn id(&self) -> SceneId {
        self.descriptor.id
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn descriptor(&self) -> SceneDescriptor {
        self.descriptor
    }

    #[must_use]
    pub fn state(&self) -> &dyn SceneState {
        self.state.as_ref()
    }

    /// Rebuilds state from the seed, discarding accumulated history
    /// (`scene_instance_rebind`, `scene.c`).
    pub fn rebind(&mut self) {
        self.state = (self.descriptor.make_state)(self.seed);
    }

    pub fn update(&mut self, frame: &SceneFrame<'_>) {
        self.state.update(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_ids_match_the_c_enum_values() {
        // These numbers are a persistence surface. If one changes, existing
        // .musi projects resolve to the wrong scene.
        assert_eq!(SceneId::Spectrum as u32, 0);
        assert_eq!(SceneId::SongAtlas as u32, 4);
        assert_eq!(SceneId::Pentagram as u32, 9);
        assert_eq!(SceneId::ALL.len(), SCENE_COUNT);
        for (index, id) in SceneId::ALL.into_iter().enumerate() {
            assert_eq!(id.index(), index);
            assert_eq!(SceneId::from_index(index), Some(id));
        }
        assert_eq!(SceneId::from_index(SCENE_COUNT), None);
    }

    #[test]
    fn stable_names_round_trip_and_are_the_c_spelling() {
        let expected = [
            "spectrum",
            "pulse",
            "orbital",
            "ascii",
            "atlas",
            "terrarium",
            "constellation",
            "cadence",
            "loom",
            "pentagram",
        ];
        for (id, name) in SceneId::ALL.into_iter().zip(expected) {
            assert_eq!(id.stable_name(), name);
            assert_eq!(SceneId::from_stable_name(name), Some(id));
        }
        assert_eq!(SceneId::from_stable_name("nope"), None);
    }

    #[test]
    fn aggregate_audio_matches_the_c_derivation() {
        // Hand-computed against plug.c:1116-1128.
        let smooth = [0.5f32, 0.25, 0.0];
        let smear = [0.1f32, 0.25, 0.4];
        let audio = SceneAudioFrame::from_spectrum(&smooth, &smear);
        assert_eq!(audio.bands_count(), 3);
        assert_eq!(audio.peak, 0.5);
        let expected_rms = ((0.25f32 + 0.0625 + 0.0) / 3.0).sqrt();
        assert!((audio.rms - expected_rms).abs() < 1.0e-6);
        // Only band 0 rises above its trail: (0.5 - 0.1) / 3.
        let expected_flux = 0.4f32 / 3.0;
        assert!((audio.spectral_flux - expected_flux).abs() < 1.0e-6);
        assert!(audio.onset, "flux 0.133 clears the 0.08 onset threshold");
    }

    #[test]
    fn an_empty_spectrum_produces_no_energy_and_no_onset() {
        let audio = SceneAudioFrame::from_spectrum(&[], &[]);
        assert_eq!(audio.rms, 0.0);
        assert_eq!(audio.spectral_flux, 0.0);
        assert!(!audio.onset);
    }

    #[test]
    fn an_instance_rebinds_deterministically() {
        let descriptor = SceneDescriptor {
            id: SceneId::Spectrum,
            state_version: 1,
            make_state: |_seed| Box::new(StatelessScene::new(SceneId::Spectrum)),
        };
        let mut instance = SceneInstance::new(descriptor, 42);
        assert_eq!(instance.id(), SceneId::Spectrum);
        assert_eq!(instance.seed(), 42);
        instance.rebind();
        assert_eq!(instance.seed(), 42);
    }
}
