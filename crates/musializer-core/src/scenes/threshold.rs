//! Threshold: a dark room, a door, and another scene through it.
//!
//! Post-legacy (2026-10-21, MV9-d), no oracle. It began as the bookends of the
//! "Door, No Room" video — a GPU Python pass that ray-traced a slate room with
//! a lavender door, light pooling on a wet floor, mist, gold motes, and the
//! Musializer render showing through the doorway — and the operator asked for
//! the compositor's work to become the application's. This is that room.
//!
//! ## The doorway shows a live scene
//!
//! `beyond` names a scene (or `0` for the doorway's own luminous fog). That
//! scene runs **inside** this one: this state owns its instance and advances
//! it with the same frame, and the drawing half renders it offscreen and
//! samples it through the doorway. Its tuning is the track's per-scene table
//! for that scene — what a treatment's `scenes` block sets.
//!
//! The doorway does not show the beyond at the size it would be at the far
//! end of a deep room. It shows the beyond *fitted to the doorway* while the
//! camera is far, and the beyond *at exactly full frame* once the doorway
//! fills the screen ([`beyond_map`]). So pushing `distance` to zero walks the
//! camera into the visualizer, and at zero the threshold draws precisely the
//! picture the beyond scene would draw on its own: a cut there is invisible.
//!
//! ## And it is handed over across that cut
//!
//! When the plan cuts from a threshold to the scene its doorway was showing,
//! [`SceneState::take_nested`] moves the running instance out, so the scene
//! on the far side of the cut is the same one, with the same state, rather
//! than a fresh one starting from nothing. The reverse cut hands it back in
//! ([`SceneState::adopt_nested`]). The composition root does both, because
//! constructing and replacing instances is its job.
//!
//! Everything here is a function of the frame and the seed: the camera's
//! sway, the door's angle and the beyond's placement are arithmetic over time
//! and settings, so a still, a clip and an export agree.

use std::any::Any;

use crate::scene::settings::index::threshold as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneInstance, SceneState};

/// The room's dimensions, in metres. The doorway is centred on x = 0 in the
/// wall plane z = 0; the room is z > 0.
pub const DOOR_WIDTH: f32 = 0.96;
pub const DOOR_HEIGHT: f32 = 2.05;
/// Eye height, which is also the doorway's vertical centre to within a few
/// centimetres: the camera looks straight through it.
pub const EYE_HEIGHT: f32 = 1.05;
/// Focal length as a multiple of the frame height (a vertical field of view
/// of about 47 degrees).
pub const FOCAL: f32 = 1.15;
/// Camera distance from the wall at `distance = 1`.
pub const FAR: f32 = 6.8;
/// How wide the door swings at `open = 1`, in degrees.
pub const MAX_ANGLE: f32 = 72.0;
/// The doorway-fitted beyond is this much taller than the doorway, so the
/// doorway never shows its edge.
pub const FIT_MARGIN: f32 = 1.22;

/// Where the camera is and where it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Radians, positive turns left.
    pub yaw: f32,
    /// Radians, positive looks up.
    pub pitch: f32,
}

/// The camera for a `distance` setting at `time_seconds`.
///
/// `distance` is mapped through a power so the last metre before the door
/// takes up a fair share of the control: a linear map would spend most of a
/// glide crossing empty floor and then lunge through the doorway. The sway is
/// a slow Lissajous drift that dies away near the door, so the crossing is
/// steady.
#[must_use]
pub fn camera(distance: f32, sway: f32, time_seconds: f64) -> Camera {
    let distance = if distance.is_finite() {
        distance.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let z = 0.02 + (FAR - 0.02) * distance.powf(1.6);
    // Zero within 30 cm of the wall: the crossing is steady and level.
    let near = ((z - 0.3) / 1.2).clamp(0.0, 1.0);
    let drift = sway.clamp(0.0, 2.0) * near;
    let t = time_seconds as f32;
    let x = 0.05 * drift * (t * 0.31).sin();
    Camera {
        x,
        y: EYE_HEIGHT + 0.03 * drift * (t * 0.23 + 1.3).sin(),
        z,
        yaw: -0.03 * x,
        pitch: -0.035 * near,
    }
}

/// The door's swing for an `open` setting, in radians.
#[must_use]
pub fn door_angle(open: f32) -> f32 {
    let open = if open.is_finite() {
        open.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (open * MAX_ANGLE).to_radians()
}

/// The scene a `beyond` setting names. `0` is the doorway's own fog, and the
/// threshold never nests itself.
#[must_use]
pub fn beyond_scene(value: f32) -> Option<SceneId> {
    if !value.is_finite() || value < 0.5 {
        return None;
    }
    let index = value.round() as usize - 1;
    SceneId::from_index(index).filter(|scene| *scene != SceneId::Threshold)
}

/// Projects a world point into a `width` by `height` frame: screen x and y in
/// pixels (origin top left) and the depth in front of the camera.
#[must_use]
pub fn project(camera: &Camera, width: f32, height: f32, point: [f32; 3]) -> (f32, f32, f32) {
    let q = [
        point[0] - camera.x,
        point[1] - camera.y,
        point[2] - camera.z,
    ];
    let (sy, cy) = (-camera.yaw).sin_cos();
    let q = [q[0] * cy + q[2] * sy, q[1], -q[0] * sy + q[2] * cy];
    let (sp, cp) = (-camera.pitch).sin_cos();
    let q = [q[0], q[1] * cp - q[2] * sp, q[1] * sp + q[2] * cp];
    let depth = -q[2];
    let z = depth.max(1.0e-3);
    let focal = FOCAL * height;
    (
        width * 0.5 + focal * q[0] / z,
        height * 0.5 - focal * q[1] / z,
        depth,
    )
}

/// Where the beyond sits on screen: its centre and its height in pixels (its
/// width is the frame's aspect times that).
///
/// Fitted to the doorway — [`FIT_MARGIN`] times the doorway's on-screen
/// height, centred on it — while the doorway is small, and exactly the frame
/// once the doorway's height reaches 2.6 frames. Between, the two blend by a
/// smoothstep of the doorway's size, so the beyond grows with the push and
/// lands on the identity before the camera passes the wall.
#[must_use]
pub fn beyond_map(camera: &Camera, width: f32, height: f32) -> (f32, f32, f32) {
    if camera.z <= 0.0 {
        return (width * 0.5, height * 0.5, height);
    }
    let (left, bottom, _) = project(camera, width, height, [-DOOR_WIDTH * 0.5, 0.0, 0.0]);
    let (right, top, _) = project(camera, width, height, [DOOR_WIDTH * 0.5, DOOR_HEIGHT, 0.0]);
    let (cx, cy) = ((left + right) * 0.5, (top + bottom) * 0.5);
    let door = (bottom - top).abs();
    let t = ((door / height - 1.0) / 1.6).clamp(0.0, 1.0);
    let k = t * t * (3.0 - 2.0 * t);
    let fitted = door * FIT_MARGIN;
    (
        cx + (width * 0.5 - cx) * k,
        cy + (height * 0.5 - cy) * k,
        fitted + (height - fitted) * k,
    )
}

/// The threshold's state: a seed, a slow light follower, and the scene in the
/// doorway.
pub struct ThresholdState {
    seed: u64,
    /// A loudness follower, fast up and slow down, so the light breathes with
    /// the music instead of flickering with every analyzer tick.
    glow: f32,
    beyond: Option<SceneInstance>,
}

impl ThresholdState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            glow: 0.0,
            beyond: None,
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The followed loudness, `0..=1`-ish.
    #[must_use]
    pub fn glow(&self) -> f32 {
        self.glow
    }

    /// The scene running in the doorway, if any.
    #[must_use]
    pub fn beyond(&self) -> Option<&SceneInstance> {
        self.beyond.as_ref()
    }
}

impl SceneState for ThresholdState {
    fn id(&self) -> SceneId {
        SceneId::Threshold
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        let target = frame.audio.rms.clamp(0.0, 1.0);
        let dt = frame.delta_seconds.clamp(0.0, 0.25);
        let rate = if target > self.glow { 6.0 } else { 1.2 };
        self.glow += (target - self.glow) * (1.0 - (-rate * dt).exp());

        let wanted = beyond_scene(frame.setting(SceneId::Threshold, control::BEYOND));
        match (wanted, self.beyond.as_ref().map(SceneInstance::id)) {
            (None, _) => self.beyond = None,
            (Some(want), Some(have)) if want == have => {}
            (Some(want), _) => {
                self.beyond = Some(SceneInstance::new(super::descriptor(want), self.seed));
            }
        }
        if let Some(beyond) = &mut self.beyond {
            beyond.update(frame);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn take_nested(&mut self) -> Option<SceneInstance> {
        self.beyond.take()
    }

    fn adopt_nested(&mut self, instance: SceneInstance) -> Option<SceneInstance> {
        if instance.id() == SceneId::Threshold || self.beyond.is_some() {
            return Some(instance);
        }
        self.beyond = Some(instance);
        None
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::Threshold,
        state_version: 1,
        make_state: |seed| Box::new(ThresholdState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::SceneSettings;

    #[test]
    fn beyond_names_a_scene_by_index_plus_one_and_never_the_threshold() {
        assert_eq!(beyond_scene(0.0), None);
        assert_eq!(beyond_scene(1.0), Some(SceneId::Spectrum));
        assert_eq!(beyond_scene(7.0), Some(SceneId::Constellation));
        assert_eq!(beyond_scene(14.0), Some(SceneId::Stage));
        assert_eq!(
            beyond_scene(15.0),
            None,
            "the threshold does not nest itself"
        );
        assert_eq!(beyond_scene(f32::NAN), None);
        // The control's bound is exactly the last nestable scene.
        let bound = crate::scene::settings::descriptor(SceneId::Threshold, control::BEYOND)
            .unwrap()
            .maximum;
        assert_eq!(
            beyond_scene(bound),
            SceneId::from_index(SceneId::Threshold.index() - 1)
        );
    }

    #[test]
    fn the_camera_walks_from_across_the_room_into_the_doorway() {
        let far = camera(1.0, 0.0, 0.0);
        let near = camera(0.0, 0.0, 0.0);
        assert!((far.z - FAR).abs() < 1.0e-4);
        assert!(
            near.z > 0.0 && near.z < 0.05,
            "in the doorway, not through the wall"
        );
        let mut last = far.z;
        for step in (0..=20).rev() {
            let z = camera(step as f32 / 20.0, 1.0, 3.0).z;
            assert!(z <= last + 1.0e-6, "monotone");
            last = z;
        }
        // The sway dies away at the door, so the crossing is steady.
        assert_eq!(camera(0.0, 2.0, 12.3).x, 0.0);
        assert!(camera(1.0, 2.0, 12.3).x.abs() > 0.0);
    }

    #[test]
    fn the_beyond_is_fitted_to_the_doorway_far_away_and_the_frame_at_the_threshold() {
        let (width, height) = (1920.0, 1080.0);
        let far = camera(1.0, 0.0, 0.0);
        let (cx, cy, h) = beyond_map(&far, width, height);
        let (_, bottom, _) = project(&far, width, height, [0.0, 0.0, 0.0]);
        let (_, top, _) = project(&far, width, height, [0.0, DOOR_HEIGHT, 0.0]);
        let door = bottom - top;
        assert!((h - door * FIT_MARGIN).abs() < 0.5, "fitted: {h} vs {door}");
        assert!((cx - width * 0.5).abs() < 1.0);
        assert!((cy - (top + bottom) * 0.5).abs() < 0.5);

        let near = camera(0.0, 0.0, 0.0);
        assert_eq!(
            beyond_map(&near, width, height),
            (width * 0.5, height * 0.5, height)
        );
        // Through the wall it is the frame by construction.
        let through = Camera { z: -0.1, ..near };
        assert_eq!(
            beyond_map(&through, width, height),
            (width * 0.5, height * 0.5, height)
        );
    }

    #[test]
    fn the_door_swings_to_its_bound_and_refuses_nonsense() {
        assert_eq!(door_angle(0.0), 0.0);
        assert!((door_angle(1.0) - MAX_ANGLE.to_radians()).abs() < 1.0e-6);
        assert_eq!(door_angle(f32::NAN), 0.0);
        assert!((door_angle(3.0) - MAX_ANGLE.to_radians()).abs() < 1.0e-6);
    }

    #[test]
    fn the_doorway_runs_the_scene_it_names_and_hands_it_over() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::Threshold, control::BEYOND, 7.0);
        let frame = SceneFrame::idle(&settings);
        let mut state = ThresholdState::new(9);
        state.update(&frame);
        assert_eq!(
            state.beyond().map(SceneInstance::id),
            Some(SceneId::Constellation)
        );

        // Handed out across a cut, and back.
        let moved = state.take_nested().expect("a running beyond");
        assert!(state.beyond().is_none());
        assert!(state.adopt_nested(moved).is_none(), "taken back");
        // A second one is refused while the doorway is occupied, and so is a
        // threshold.
        let spare = SceneInstance::new(crate::scenes::descriptor(SceneId::Loom), 1);
        assert!(state.adopt_nested(spare).is_some());
        let mut empty = ThresholdState::new(1);
        let nested = SceneInstance::new(descriptor(), 1);
        assert!(empty.adopt_nested(nested).is_some());

        // Naming a different scene replaces it; naming none drops it.
        settings.set(SceneId::Threshold, control::BEYOND, 9.0);
        state.update(&SceneFrame::idle(&settings));
        assert_eq!(state.beyond().map(SceneInstance::id), Some(SceneId::Loom));
        settings.set(SceneId::Threshold, control::BEYOND, 0.0);
        state.update(&SceneFrame::idle(&settings));
        assert!(state.beyond().is_none());
    }
}
