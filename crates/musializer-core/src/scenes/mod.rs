//! Deterministic scene state and per-frame updates.
//!
//! **Owned per file:** Agent C owns the first five, Agent D the last five.
//! This `mod.rs` belongs to the integration owner — it is pre-declared so two
//! agents never edit it at once. The drawing halves live in
//! `musializer-app::scenes`.
//!
//! Every scene here must be deterministic for the same seed, settings, and
//! frame sequence: no wall clock, no unseeded RNG, no I/O.

pub mod ascii_field;
pub mod cadence;
/// Post-legacy (2026-08-24), no oracle. See its module docs for its evidence.
pub mod clawd;
pub mod constellation;
pub mod loom;
pub mod orbital_lattice;
pub mod pentagram;
/// The one scene here with no oracle behind it: a post-legacy addition rather
/// than a port. See its module docs for what that changes about its evidence.
pub mod phosphor_dream;
pub mod pulse_field;
pub mod song_atlas;
pub mod spectral_terrarium;
pub mod spectrum;

pub mod openai_knot;
pub mod stage;
/// Post-legacy (2026-10-21, MV9-d): a room, a door, and a scene nested in it.
pub mod threshold;

use crate::scene::{SceneDescriptor, SceneId, StatelessScene};

/// The registry: each scene's descriptor.
///
/// Lived in `musializer-app::scene_host` until a scene needed to construct
/// another (the threshold runs a scene inside its doorway). Every arm already
/// built a type from this crate, so moving the table here changes no state;
/// `scene_host::descriptor` delegates.
#[must_use]
pub fn descriptor(id: SceneId) -> SceneDescriptor {
    let simple = |state_version, make_state| SceneDescriptor {
        id,
        state_version,
        make_state,
    };
    match id {
        // Spectrum is a pure function of the frame.
        SceneId::Spectrum => simple(1, |_| Box::new(StatelessScene::new(SceneId::Spectrum))),
        SceneId::PulseField => simple(1, |seed| Box::new(pulse_field::PulseFieldState::new(seed))),
        SceneId::OrbitalLattice => simple(1, |seed| {
            Box::new(orbital_lattice::OrbitalLatticeState::new(seed))
        }),
        SceneId::AsciiField => simple(1, |seed| Box::new(ascii_field::AsciiFieldState::new(seed))),
        SceneId::SongAtlas => simple(1, |seed| Box::new(song_atlas::SongAtlasState::new(seed))),
        SceneId::SpectralTerrarium => simple(1, |seed| {
            Box::new(spectral_terrarium::SpectralTerrariumState::new(seed))
        }),
        SceneId::Constellation => simple(1, |seed| {
            Box::new(constellation::ConstellationState::new(seed))
        }),
        SceneId::Cadence => simple(1, |seed| Box::new(cadence::CadenceState::new(seed))),
        SceneId::Loom => simple(1, |seed| Box::new(loom::LoomState::new(seed))),
        SceneId::Pentagram => simple(1, |seed| Box::new(pentagram::PentagramState::new(seed))),
        SceneId::PhosphorDream => phosphor_dream::descriptor(),
        SceneId::Clawd => clawd::descriptor(),
        SceneId::OpenAiKnot => openai_knot::descriptor(),
        SceneId::Stage => stage::descriptor(),
        SceneId::Threshold => threshold::descriptor(),
    }
}
