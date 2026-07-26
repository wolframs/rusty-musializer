//! Spectrum: the registry half. There is no deterministic state.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_spectrum.c` (frozen at
//! `9300af9`, read-only).
//!
//! The C descriptor sets `.id`, `.name`, `.state_version` and `.draw` and
//! nothing else (`scene_spectrum.c:149-154`): no `state_size`, no `init`, no
//! `update`. Spectrum is a pure function of the frame, so the only thing this
//! file owes the registry is a descriptor over [`StatelessScene`]. The drawing
//! half is `musializer-app::scenes::spectrum`.

use crate::scene::{SceneDescriptor, SceneId, StatelessScene};

/// `scene_spectrum.c:152`.
pub const STATE_VERSION: u32 = 1;

/// The registry entry (`scene_spectrum.c:149-154`).
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::Spectrum,
        state_version: STATE_VERSION,
        make_state: |_seed| Box::new(StatelessScene::new(SceneId::Spectrum)),
    }
}

/// Spectrum's state factory, for callers that want the concrete type.
///
/// The seed is deliberately ignored: the C `init` pointer is null, so there is
/// nothing for a seed to vary.
#[must_use]
pub fn make_state(_seed: u64) -> StatelessScene {
    StatelessScene::new(SceneId::Spectrum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneFrame, SceneInstance, SceneSettings};

    #[test]
    fn the_descriptor_matches_the_c_one() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SceneId::Spectrum);
        assert_eq!(descriptor.state_version, 1);
        assert_eq!(descriptor.name(), "Spectrum");
        assert_eq!(descriptor.stable_name(), "spectrum");
    }

    #[test]
    fn updating_a_stateless_scene_is_a_no_op_for_any_seed() {
        let settings = SceneSettings::default();
        let frame = SceneFrame::idle(&settings);
        // Two different seeds must be indistinguishable, because C never gives
        // Spectrum an `init` to consume one.
        for seed in [0u64, 1, u64::MAX] {
            let mut instance = SceneInstance::new(descriptor(), seed);
            instance.update(&frame);
            assert_eq!(instance.state().id(), SceneId::Spectrum);
        }
    }
}
