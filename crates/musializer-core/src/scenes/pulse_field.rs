//! Pulse Field: deterministic state and update.
//!
//! **Owner: Agent C.** Port of `../musializer/src/scene_pulse_field.c` (frozen
//! at `9300af9`, read-only).
//!
//! The whole state is one number: a per-instance rotation offset taken from the
//! seed (`scene_pulse_field.c:5-13`). The C `update` exists in the descriptor but
//! its body is `(void)state; (void)frame;` — every other animated quantity is
//! derived from `frame->time_seconds` inside `draw`. That is reproduced rather
//! than "improved": moving the animation into `update` would change what a
//! seeked preview shows.

use std::any::Any;

use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};

/// `scene_pulse_field.c:127`.
pub const STATE_VERSION: u32 = 1;

/// `Pulse_Field_State` (`scene_pulse_field.c:5-7`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulseFieldState {
    /// Degrees of starting rotation for this instance's rose.
    pub rotation: f32,
}

impl PulseFieldState {
    /// `pulse_field_init` (`scene_pulse_field.c:9-13`): `seed % 360` degrees.
    ///
    /// The modulo runs in `uint64_t` and only then converts, so a seed of
    /// `u64::MAX` gives 15 degrees rather than saturating.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            rotation: (seed % 360) as f32,
        }
    }
}

impl SceneState for PulseFieldState {
    fn id(&self) -> SceneId {
        SceneId::PulseField
    }

    /// Empty, exactly as `pulse_field_update` is (`scene_pulse_field.c:15-19`).
    fn update(&mut self, _frame: &SceneFrame<'_>) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The registry entry (`scene_pulse_field.c:124-132`).
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::PulseField,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(PulseFieldState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneInstance, SceneSettings};

    #[test]
    fn the_seed_becomes_a_rotation_in_degrees() {
        assert_eq!(PulseFieldState::new(0).rotation, 0.0);
        assert_eq!(PulseFieldState::new(359).rotation, 359.0);
        assert_eq!(PulseFieldState::new(360).rotation, 0.0);
        assert_eq!(PulseFieldState::new(721).rotation, 1.0);
        // u64::MAX % 360 == 15. The C does the modulo in 64-bit before the
        // conversion to float, so this is not a saturating cast.
        assert_eq!(PulseFieldState::new(u64::MAX).rotation, 15.0);
    }

    #[test]
    fn update_changes_nothing_so_a_seek_cannot_drift() {
        let settings = SceneSettings::default();
        let mut state = PulseFieldState::new(97);
        let before = state;
        for index in 0..120u64 {
            let frame = SceneFrame {
                time_seconds: index as f64 / 60.0,
                delta_seconds: 1.0 / 60.0,
                frame_index: index,
                ..SceneFrame::idle(&settings)
            };
            state.update(&frame);
        }
        assert_eq!(state, before);
    }

    #[test]
    fn the_descriptor_matches_the_c_one() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SceneId::PulseField);
        assert_eq!(descriptor.state_version, 1);
        assert_eq!(descriptor.name(), "Pulse Field");
        let instance = SceneInstance::new(descriptor, 42);
        let state = instance
            .state()
            .as_any()
            .downcast_ref::<PulseFieldState>()
            .expect("the descriptor builds a PulseFieldState");
        assert_eq!(state.rotation, 42.0);
    }
}
