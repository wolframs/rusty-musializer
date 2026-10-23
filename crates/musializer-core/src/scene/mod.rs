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

/// The scenes, in registry order (`../musializer/src/scene.h:17-29` for 0..=9).
///
/// The discriminants are load-bearing: `.musi` projects and the `--scene` CLI
/// flag both resolve through them. Ids 0..=9 are the frozen C's own enum values
/// and may never move.
///
/// [`PhosphorDream`] is the first scene with no oracle behind it at all — a
/// post-legacy addition (2026-08-08) rather than a port, appended at id 10 so
/// every C-era id keeps its meaning and every project written before it still
/// opens. See [`SCENE_COUNT`] for what an eleventh scene costs.
///
/// [`PhosphorDream`]: SceneId::PhosphorDream
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
    /// Post-legacy, no C counterpart (2026-08-08).
    PhosphorDream = 10,
    /// Post-legacy, no C counterpart (2026-08-24).
    Clawd = 11,
    /// Procedural sixfold weave (2026-09-09).
    OpenAiKnot = 12,
    /// The lyric line, staged (2026-10-12).
    Stage = 13,
    /// A room, a door, and another scene through it (2026-10-21, MV9-d).
    Threshold = 14,
    /// Music-stirred luminous dye (2026-10-23).
    FluidInk = 15,
    LiquidChrome = 16,
    AuroraVeil = 17,
    MetaballGarden = 18,
    NeonTunnel = 19,
    VoronoiBloom = 20,
    IridescentPool = 21,
}

/// Number of scenes.
///
/// This was C's `COUNT_SCENES`/`SCENE_SETTINGS_SCENE_COUNT` (`scene.h:28`,
/// `scene_settings_values.h:8`) and equalled 10 for as long as the rewrite was
/// chasing parity. Post-legacy scenes are appended to the registry; the current count is derived
/// from that registry below. This is a deliberate divergence under the
/// 2026-08-03 legacy decision rather than a drift.
///
/// What it moves, recorded here because the number is load-bearing in four
/// places a reader would not guess:
///
/// - [`settings::SceneSettings`] is a dense `[[f32; MAX_CONTROLS]; SCENE_COUNT]`,
///   so it grows by one row. Row 10 is unreachable from any pre-existing file.
/// - [`routes::RouteTable`] grows the same way.
/// - `project::model::MAX_MAPPINGS_PER_SCENE` and `MAX_SCENE_PRESETS` are
///   derived from it and grow with it — both are ceilings, so raising them
///   cannot reject a file that used to load.
/// - The differential harnesses compare the **C-era ten** and pin the eleventh
///   separately; see `tools/differential_settings.sh`.
///
/// Nothing here is visible to a `.musi` file written before 2026-08-08: a
/// project stores per-scene data by token (`settings.loom.weight`), not by
/// count, so a shorter table is simply a table with no `phosphor` rows in it.
pub const SCENE_COUNT: usize = SceneId::IridescentPool as usize + 1;

/// The scenes the frozen C had, which is the prefix every differential harness
/// compares against (`scene.h:17-29`).
///
/// Named rather than written as a literal 10 in the harness dumps, so that
/// "how many scenes does the oracle have" and "how many do we have" can never
/// be confused for each other again.
pub const ORACLE_SCENE_COUNT: usize = 10;

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
        SceneId::PhosphorDream,
        SceneId::Clawd,
        SceneId::OpenAiKnot,
        SceneId::Stage,
        SceneId::Threshold,
        SceneId::FluidInk,
        SceneId::LiquidChrome,
        SceneId::AuroraVeil,
        SceneId::MetaballGarden,
        SceneId::NeonTunnel,
        SceneId::VoronoiBloom,
        SceneId::IridescentPool,
    ];

    /// The C-era prefix of [`Self::ALL`], for the differential harnesses.
    pub const ORACLE_ALL: [SceneId; ORACLE_SCENE_COUNT] = [
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

    /// Whether the frozen C has this scene at all.
    ///
    /// The harnesses read this rather than comparing against a literal 10, and
    /// so does the report line that names which scene drew a capture.
    #[must_use]
    pub fn exists_in_oracle(self) -> bool {
        self.index() < ORACLE_SCENE_COUNT
    }

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
            SceneId::PhosphorDream => "Phosphor Dream",
            SceneId::Clawd => "Clawd",
            SceneId::OpenAiKnot => "OpenAI Knot",
            SceneId::Stage => "Stage",
            SceneId::Threshold => "Threshold",
            SceneId::FluidInk => "Fluid Ink",
            SceneId::LiquidChrome => "Liquid Chrome",
            SceneId::AuroraVeil => "Aurora Veil",
            SceneId::MetaballGarden => "Metaball Garden",
            SceneId::NeonTunnel => "Neon Tunnel",
            SceneId::VoronoiBloom => "Voronoi Bloom",
            SceneId::IridescentPool => "Iridescent Pool",
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
            // Not "dreamscape": the piece this scene grew out of is named after a
            // copyrighted track, and a persisted token is forever. "phosphor" is
            // what the scene actually is.
            SceneId::PhosphorDream => "phosphor",
            // The character's community spelling, not "claude": the token is a
            // persisted identifier and a scene is not the assistant. "clawd" is
            // the name thebes' flower answers to.
            SceneId::Clawd => "clawd",
            SceneId::OpenAiKnot => "knot",
            SceneId::Stage => "stage",
            SceneId::Threshold => "threshold",
            SceneId::FluidInk => "fluid",
            SceneId::LiquidChrome => "chrome",
            SceneId::AuroraVeil => "aurora",
            SceneId::MetaballGarden => "metaballs",
            SceneId::NeonTunnel => "tunnel",
            SceneId::VoronoiBloom => "voronoi",
            SceneId::IridescentPool => "iridescent",
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

    /// Broad, stable spectral regions for parameter routing and scene design.
    ///
    /// The analyzer bands are logarithmically spaced and already temporally
    /// smoothed. The lowest and highest fifths preserve Pulse Field's original
    /// bass/treble definition; the middle is everything between them. Keeping
    /// this derivation here gives every scene and route the same musical terms
    /// instead of letting each renderer invent slightly different cutoffs.
    #[must_use]
    pub fn spectral_regions(&self) -> (f32, f32, f32) {
        if self.bands.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let edge_count = (self.bands.len() / 5).max(1);
        let bass = mean(&self.bands[..edge_count]);
        let treble = mean(&self.bands[self.bands.len() - edge_count..]);
        let mids = if edge_count * 2 >= self.bands.len() {
            mean(self.bands)
        } else {
            mean(&self.bands[edge_count..self.bands.len() - edge_count])
        };
        (bass, mids, treble)
    }

    /// Treble's share of the bass-plus-treble energy, from bass-heavy `0` to
    /// treble-heavy `1`.
    ///
    /// The small denominator floor is the original Pulse Field formula. Silence
    /// therefore settles at zero instead of producing a non-finite route value.
    #[must_use]
    pub fn spectral_balance(&self) -> f32 {
        let (bass, _, treble) = self.spectral_regions();
        1.0f32.min(treble / (bass + treble + 0.001))
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

    /// Fills in [`Self::beat_phase`] from a tracker (`plug.c:1139-1144`).
    ///
    /// Separate from [`Self::from_spectrum`] because the tracker is stateful and
    /// this type is not, but it belongs *next* to it: `beat_phase` is a documented
    /// route source, so a caller that builds a frame and forgets this leaves
    /// `--route parameter:beat_phase:...` evaluating a constant zero forever. That
    /// is exactly what happened — the tracker was ported with tests and had no
    /// caller at all, while the CLI advertised the source.
    ///
    /// Preview and export must both call it, which is the reason it is one method
    /// on the shared type rather than two copies of four lines: routed parameters
    /// staying preview/export identical is a stated invariant, and a beat that only
    /// advanced in the preview would break it silently.
    ///
    /// A rejected update **resets** the tracker rather than being ignored
    /// (`plug.c:1141-1143`). The tracker refuses a non-monotonic or non-finite
    /// time, which is precisely the seek case, and a stale anchor would otherwise
    /// keep producing a phase that no longer refers to the audio.
    ///
    /// The phase used on a rejection is *not* simply zero, and getting that wrong
    /// was a parity bug the `beat_tracker` differential harness found. The C keeps a
    /// local `float beat_phase = 0.0f;` and passes its address, so:
    ///
    /// - refused input → the tracker never writes, and the zero stands;
    /// - a phase that narrowed to exactly 1.0 → the tracker writes it and *then*
    ///   returns false, so the local holds 1.0, and `plug.c:1157`/`plug.c:1185`
    ///   copy it into the scene frame anyway.
    ///
    /// [`phase_or_caller_default`] is that distinction; it is a named method rather
    /// than an `unwrap_or(0.0)` here because the zero belongs to the C caller's
    /// initialiser rather than being a neutral fallback.
    ///
    /// [`phase_or_caller_default`]:
    ///     crate::audio::beat_tracker::BeatUpdate::phase_or_caller_default
    pub fn track_beat(
        &mut self,
        tracker: &mut crate::audio::beat_tracker::BeatTracker,
        time_seconds: f64,
    ) {
        let update = tracker.update(time_seconds, self.onset, self.spectral_flux);
        self.beat_phase = update.phase_or_caller_default();
        if !update.is_usable() {
            tracker.reset();
        }
    }
}

fn mean(values: &[f32]) -> f32 {
    values.iter().copied().sum::<f32>() / values.len() as f32
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
    /// The look of the voice singing this line, when the project names one
    /// (post-legacy, MV9-h). Cadence and Stage take its hue in place of their
    /// own `hue` setting; the caption overlay takes its ink and glow.
    pub voice: Option<VoiceStyle>,
}

/// How one voice's lines look (post-legacy, MV9-h). Every member optional: a
/// voice can turn only the hue, or only the caption's ink.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoiceStyle {
    /// The hue of the colour the voice's lines are drawn in, in degrees on the
    /// colour wheel, `0..=360` (0 red, 60 yellow, 120 green, 240 blue, 270
    /// violet). Absolute rather than a scene's own `hue` offset, because every
    /// scene's offset starts from a different colour: Cadence's ink at offset
    /// 0 is pink and Stage's accent is lime, so one offset would paint one
    /// voice two different colours. Each scene maps it with its own base.
    pub hue: Option<f32>,
    /// `0xRRGGBBAA` caption ink.
    pub text_rgba: Option<u32>,
    /// `0xRRGGBBAA` caption glow colour.
    pub glow_rgba: Option<u32>,
}

impl VoiceStyle {
    /// Whether this is a style a project may hold.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.hue
            .is_none_or(|hue| hue.is_finite() && (0.0..=360.0).contains(&hue))
    }

    /// A scene's `hue` offset: the one that puts this voice's colour where the
    /// scene puts the colour its offset 0 draws at `base` degrees, when the line
    /// has a voice with a hue; else the scene's own setting. Wrapped into
    /// `-180..180`, the range every scene's hue control has.
    #[must_use]
    pub fn hue_or(voice: Option<&Self>, own: f32, base: f32) -> f32 {
        match voice.and_then(|voice| voice.hue) {
            Some(hue) => (hue - base + 540.0).rem_euclid(360.0) - 180.0,
            None => own,
        }
    }
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
    /// The track's own loudness range (post-legacy, 2026-08-26), profiled once
    /// from the whole decoded file. `None` before the profile exists — idle,
    /// a failed decode, a near-silent file — and a scene that gates behaviour
    /// on it must fall back to absolute constants *and report which it used*:
    /// an absent profile is indistinguishable from a wired one on a loud track,
    /// which is the fallback-that-looks-like-content trap.
    pub dynamics: Option<crate::audio::track_dynamics::TrackDynamics>,
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
            dynamics: None,
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

    /// Hands out a scene this one is running inside itself, if it has one
    /// (MV9-d). Only the threshold does: when the plan cuts from it to the
    /// scene its doorway was showing, the running instance moves out instead
    /// of a fresh one starting, so the cut does not restart the picture.
    fn take_nested(&mut self) -> Option<SceneInstance> {
        None
    }

    /// Offers this scene a running instance to show inside itself (MV9-d),
    /// the reverse hand-over. Hands it back when the scene does not want it —
    /// which every scene but the threshold, and the threshold showing a
    /// different scene, does.
    fn adopt_nested(&mut self, instance: SceneInstance) -> Option<SceneInstance> {
        Some(instance)
    }
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

    /// [`SceneState::take_nested`] on this instance's state.
    pub fn take_nested(&mut self) -> Option<SceneInstance> {
        self.state.take_nested()
    }

    /// [`SceneState::adopt_nested`] on this instance's state.
    pub fn adopt_nested(&mut self, instance: SceneInstance) -> Option<SceneInstance> {
        self.state.adopt_nested(instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three outcomes of a tracker update reach [`SceneAudioFrame::beat_phase`]
    /// differently, and this is the layer where the difference is observable: a
    /// route reading `beat_phase` renders it.
    ///
    /// Pinned here rather than only in `beat_tracker`'s own tests because the bug
    /// was in *this* function. It collapsed a refused input and an out-of-range
    /// phase into the same zero, where `plug.c:1139-1144` distinguishes them by
    /// keeping a local and passing its address.
    #[test]
    fn track_beat_reproduces_the_c_callers_three_outcomes() {
        use crate::audio::beat_tracker::BeatTracker;

        let mut tracker = BeatTracker::new();
        let mut frame = SceneAudioFrame::default();

        // A usable phase: 0.125 s into the neutral 0.5 s clock is a quarter beat.
        frame.track_beat(&mut tracker, 0.0);
        assert_eq!(frame.beat_phase, 0.0);
        frame.track_beat(&mut tracker, 0.125);
        assert_eq!(frame.beat_phase, 0.25);
        assert_eq!(tracker.learned_intervals(), 0);

        // A phase that narrows to exactly 1.0: refused by the tracker, and the C's
        // caller uses it anyway. Not 0.0.
        frame.track_beat(&mut tracker, 0.499_999_999_995);
        assert_eq!(
            frame.beat_phase, 1.0,
            "the C's local holds the value the tracker wrote before refusing it"
        );

        // A refused *input*: the tracker never writes, so the C's local keeps the
        // 0.0 it was initialised with.
        frame.beat_phase = 0.5;
        frame.track_beat(&mut tracker, f64::NAN);
        assert_eq!(frame.beat_phase, 0.0);
    }

    #[test]
    fn a_voice_colour_maps_into_each_scenes_own_hue_offset() {
        let lavender = VoiceStyle {
            hue: Some(253.0),
            ..VoiceStyle::default()
        };
        let gold = VoiceStyle {
            hue: Some(33.0),
            ..VoiceStyle::default()
        };
        // Cadence draws its ink at 323 + offset: the offsets the Door video
        // used by hand for its two voices.
        assert!((VoiceStyle::hue_or(Some(&lavender), 0.0, 323.0) + 70.0).abs() < 1.0e-4);
        assert!((VoiceStyle::hue_or(Some(&gold), 0.0, 323.0) - 70.0).abs() < 1.0e-4);
        // Stage's accent is at 68 + offset: one voice, one colour, two scenes.
        assert!((VoiceStyle::hue_or(Some(&lavender), 0.0, 68.0) + 175.0).abs() < 1.0e-4);
        // No voice, or a voice with no hue: the scene's own setting.
        assert_eq!(VoiceStyle::hue_or(None, 12.0, 323.0), 12.0);
        assert_eq!(
            VoiceStyle::hue_or(Some(&VoiceStyle::default()), 12.0, 323.0),
            12.0
        );
        assert!(!VoiceStyle {
            hue: Some(361.0),
            ..VoiceStyle::default()
        }
        .is_valid());
    }

    #[test]
    fn scene_ids_match_the_c_enum_values() {
        // These numbers are a persistence surface. If one changes, existing
        // .musi projects resolve to the wrong scene.
        assert_eq!(SceneId::Spectrum as u32, 0);
        assert_eq!(SceneId::SongAtlas as u32, 4);
        assert_eq!(SceneId::Pentagram as u32, 9);
        // Appended, not inserted: every C-era id above keeps its value, which is
        // what lets a .musi written before 2026-08-08 still resolve its scenes.
        assert_eq!(SceneId::PhosphorDream as u32, 10);
        assert_eq!(SceneId::Clawd as u32, 11);
        assert_eq!(SceneId::OpenAiKnot as u32, 12);
        assert_eq!(SceneId::Stage as u32, 13);
        assert_eq!(SceneId::Threshold as u32, 14);
        assert_eq!(SceneId::FluidInk as u32, 15);
        assert_eq!(SceneId::LiquidChrome as u32, 16);
        assert_eq!(SceneId::AuroraVeil as u32, 17);
        assert_eq!(SceneId::MetaballGarden as u32, 18);
        assert_eq!(SceneId::NeonTunnel as u32, 19);
        assert_eq!(SceneId::VoronoiBloom as u32, 20);
        assert_eq!(SceneId::IridescentPool as u32, 21);
        assert_eq!(SceneId::ALL.len(), SCENE_COUNT);
        assert_eq!(SceneId::ORACLE_ALL.len(), ORACLE_SCENE_COUNT);
        assert_eq!(SceneId::ALL[..ORACLE_SCENE_COUNT], SceneId::ORACLE_ALL);
        for id in SceneId::ORACLE_ALL {
            assert!(id.exists_in_oracle());
        }
        assert!(!SceneId::PhosphorDream.exists_in_oracle());
        assert!(!SceneId::Clawd.exists_in_oracle());
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
            "phosphor",
            "clawd",
            "knot",
            "stage",
            "threshold",
            "fluid",
            "chrome",
            "aurora",
            "metaballs",
            "tunnel",
            "voronoi",
            "iridescent",
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
        assert_eq!(audio.spectral_regions(), (0.0, 0.0, 0.0));
        assert_eq!(audio.spectral_balance(), 0.0);
    }

    #[test]
    fn spectral_regions_preserve_pulses_fifths_and_balance() {
        let bands = [0.2, 0.4, 0.3, 0.5, 0.7, 0.9, 0.6, 0.8, 0.1, 1.0];
        let audio = SceneAudioFrame {
            bands: &bands,
            ..SceneAudioFrame::default()
        };
        let (bass, mids, treble) = audio.spectral_regions();
        assert!((bass - 0.3).abs() < 1.0e-6);
        assert!((mids - (3.8 / 6.0)).abs() < 1.0e-6);
        assert!((treble - 0.55).abs() < 1.0e-6);
        assert!((audio.spectral_balance() - (0.55 / 0.851)).abs() < 1.0e-6);

        let single = SceneAudioFrame {
            bands: &[0.75],
            ..SceneAudioFrame::default()
        };
        assert_eq!(single.spectral_regions(), (0.75, 0.75, 0.75));
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
