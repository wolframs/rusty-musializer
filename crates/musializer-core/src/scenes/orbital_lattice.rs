//! Orbital Lattice: deterministic state and update.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_orbital_lattice.c`'s
//! `init`/`update` half (frozen at `9300af9`, read-only). The whole of the state
//! is [`motion::Motion`], which is the port of the separate raylib-free
//! `scene_orbital_lattice_motion.c` module; the drawing half is
//! `musializer-app::scenes::orbital_lattice`.
//!
//! C reuses `Orbital_Lattice_Motion` *as* the scene state — `init` is just
//! `orbital_lattice_motion_init(state, seed)` (`scene_orbital_lattice.c:62-65`).
//! This keeps that shape: the state is a newtype over the motion so the trait
//! impl has somewhere to live without a second copy of the fields.

pub mod motion;

use std::any::Any;

use crate::scene::settings::index::orbital as setting;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

pub use motion::{
    Motion, MotionInput, RingMotion, NODES_PER_RING, PATH_LENGTH, RING_COUNT, RING_SPACING,
};

/// `scene_orbital_lattice.c:317`. Version 2, not 1: the state layout changed
/// once, so a project carrying version-1 state must be rebound rather than
/// reinterpreted.
pub const STATE_VERSION: u32 = 2;

/// The scene state (`scene_orbital_lattice.c:318` — `sizeof(Orbital_Lattice_Motion)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitalLatticeState {
    motion: Motion,
}

impl OrbitalLatticeState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            motion: Motion::new(seed),
        }
    }

    /// The damped envelopes and phases the drawing half reads.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        &self.motion
    }
}

impl SceneState for OrbitalLatticeState {
    fn id(&self) -> SceneId {
        SceneId::OrbitalLattice
    }

    /// `orbital_lattice_update` (`scene_orbital_lattice.c:67-86`).
    ///
    /// Note which setting reaches the motion module: only `ORBITAL_SETTING_MOTION`,
    /// as the rate multiplier. Radius, depth, tilt, reactivity and sway are read
    /// in `draw` instead, so changing them cannot alter accumulated state — which
    /// is what lets the Tune panel be scrubbed without the lattice jumping.
    fn update(&mut self, frame: &SceneFrame<'_>) {
        let input = MotionInput {
            time_seconds: frame.time_seconds,
            delta_seconds: frame.delta_seconds,
            bands: frame.audio.bands,
            rms: frame.audio.rms,
            spectral_flux: frame.audio.spectral_flux,
            onset: frame.audio.onset,
            semantic_available: frame.semantic.available,
            semantic_valence: frame.semantic.valence,
            semantic_tension: frame.semantic.tension,
            semantic_confidence: frame.semantic.confidence,
            motion_rate: frame.setting(SceneId::OrbitalLattice, setting::MOTION),
        };
        self.motion.update(&input);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The registry entry (`scene_orbital_lattice.c:314-322`).
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::OrbitalLattice,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(OrbitalLatticeState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneAudioFrame, SceneSettings};

    #[test]
    fn the_descriptor_matches_the_c_one() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SceneId::OrbitalLattice);
        assert_eq!(descriptor.state_version, 2);
        assert_eq!(descriptor.name(), "Orbital Lattice");
    }

    #[test]
    fn the_frame_drives_the_motion_module_and_the_seed_reaches_it() {
        let settings = SceneSettings::default();
        let smooth = [0.9f32, 0.8, 0.4, 0.2, 0.1, 0.05];
        let smear = [0.1f32; 6];
        let mut state = OrbitalLatticeState::new(1234);
        assert!(!state.motion().initialized);
        assert_eq!(state.motion().seed, 1234);

        let frame = SceneFrame {
            time_seconds: 1.0,
            delta_seconds: 1.0 / 60.0,
            frame_index: 60,
            audio: SceneAudioFrame::from_spectrum(&smooth, &smear),
            ..SceneFrame::idle(&settings)
        };
        state.update(&frame);
        assert!(state.motion().initialized);
        // Bass-heavy input, so the low envelope leads.
        assert!(state.motion().bass > state.motion().treble);
        assert!(state.motion().ring(0).is_some());
    }

    #[test]
    fn the_motion_setting_scales_phase_and_nothing_else_does() {
        let mut settings = SceneSettings::default();
        let quiet = [0.5f32; 8];
        let mut slow = OrbitalLatticeState::new(7);
        let mut fast = OrbitalLatticeState::new(7);

        fn make<'a>(settings: &'a SceneSettings, quiet: &'a [f32]) -> SceneFrame<'a> {
            SceneFrame {
                time_seconds: 4.0,
                delta_seconds: 1.0 / 60.0,
                audio: SceneAudioFrame::from_spectrum(quiet, quiet),
                ..SceneFrame::idle(settings)
            }
        }
        slow.update(&make(&settings, &quiet));
        assert!(settings.set(SceneId::OrbitalLattice, setting::MOTION, 2.0));
        fast.update(&make(&settings, &quiet));
        assert_ne!(slow.motion().travel_phase, fast.motion().travel_phase);

        // Radius is a draw-time setting: it must not touch state at all.
        let mut with_radius = OrbitalLatticeState::new(7);
        let mut other = settings;
        assert!(other.set(SceneId::OrbitalLattice, setting::RADIUS, 1.45));
        with_radius.update(&make(&other, &quiet));
        assert_eq!(with_radius.motion(), fast.motion());
    }
}
