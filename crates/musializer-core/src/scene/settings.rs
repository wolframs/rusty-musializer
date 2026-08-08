//! Per-scene tunable settings: the descriptor table, the value store, and
//! snapshot compatibility.
//!
//! **Shared contract.** Consumed by Agents C, D and F. Port of
//! `../musializer/src/scene_settings.c`, `.h`, and `scene_settings_values.h`.
//!
//! Every bound and default in [`descriptors`] is copied from
//! `scene_settings.c:13-141`. They are a compatibility surface twice over: a
//! `.musi` file stores values that must still validate, and a value out of range
//! silently becomes the default rather than being clamped, so a wrong bound here
//! shows up as a scene that quietly ignores a saved setting.

use super::{SceneId, SCENE_COUNT};

/// Largest control count any scene has (`scene_settings_values.h:9`).
/// Song Atlas has twelve, which is the current maximum.
pub const MAX_CONTROLS: usize = 12;
/// Presets stored per scene in the per-user library (`scene_settings_values.h:10`).
pub const PRESETS_PER_SCENE: usize = 8;
/// Preset name buffer size in C, including its NUL (`scene_settings.h:11`).
pub const PRESET_NAME_CAPACITY: usize = 129;

/// Slider or toggle (`scene_settings.h:14-17`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Slider,
    /// A toggle's value must be exactly 0.0 or 1.0 — not merely within range
    /// (`scene_settings.c:147-148`).
    Toggle,
}

/// One tunable control (`scene_settings.h:55-63`).
#[derive(Clone, Copy, Debug)]
pub struct SettingDescriptor {
    /// The persisted parameter key, e.g. `"settings.loom.weight"`. A `.musi`
    /// compatibility surface — never rename one.
    pub key: &'static str,
    /// The label the inspector shows.
    pub label: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub default_value: f32,
    /// Decimal places the UI renders. `0` means the control reads as an integer.
    pub precision: u32,
    pub kind: SettingKind,
}

impl SettingDescriptor {
    const fn slider(
        key: &'static str,
        label: &'static str,
        minimum: f32,
        maximum: f32,
        default_value: f32,
        precision: u32,
    ) -> Self {
        Self {
            key,
            label,
            minimum,
            maximum,
            default_value,
            precision,
            kind: SettingKind::Slider,
        }
    }

    /// A toggle is a 0..1 slider with precision 0 whose value must be exactly
    /// 0 or 1 (`scene_settings.c:10-11`).
    const fn toggle(key: &'static str, label: &'static str, default_value: f32) -> Self {
        Self {
            key,
            label,
            minimum: 0.0,
            maximum: 1.0,
            default_value,
            precision: 0,
            kind: SettingKind::Toggle,
        }
    }

    /// The in-range test (`scene_settings.c:143-149`).
    #[must_use]
    pub fn accepts(&self, value: f32) -> bool {
        if !value.is_finite() || value < self.minimum || value > self.maximum {
            return false;
        }
        self.kind != SettingKind::Toggle || value == 0.0 || value == 1.0
    }
}

/// Setting indices per scene. These are positional and persisted through
/// snapshots, so appending is safe and reordering is not.
pub mod index {
    /// `scene_settings.h:19-22`
    pub mod spectrum {
        pub const AMPLITUDE: usize = 0;
        pub const TRAIL: usize = 1;
        pub const SATURATION: usize = 2;
        pub const GLOW_SOFTNESS: usize = 3;
        pub const HUE_SWING: usize = 4;
        pub const CORE_GLOW: usize = 5;
        pub const BAR_TAPER: usize = 6;
        pub const REFLECTION: usize = 7;
    }
    /// `scene_settings.h:23-25`
    pub mod pulse {
        pub const SCALE: usize = 0;
        pub const RINGS: usize = 1;
        pub const MOTION: usize = 2;
        pub const ARC: usize = 3;
        pub const WEIGHT: usize = 4;
        pub const PETALS: usize = 5;
        pub const HUE: usize = 6;
        pub const GLOW: usize = 7;
    }
    /// `scene_settings.h:26-28`
    pub mod orbital {
        pub const MOTION: usize = 0;
        pub const RADIUS: usize = 1;
        pub const DEPTH: usize = 2;
        pub const NODES: usize = 3;
        pub const LINKS: usize = 4;
        pub const TILT: usize = 5;
        pub const HUE: usize = 6;
        pub const REACTIVITY: usize = 7;
        pub const SWAY: usize = 8;
    }
    /// `scene_settings.h:29-30`
    pub mod ascii {
        pub const MOTION: usize = 0;
        pub const CYCLING: usize = 1;
        pub const SCANLINES: usize = 2;
        pub const SPLIT: usize = 3;
        pub const GAIN: usize = 4;
        pub const TINT: usize = 5;
    }
    /// `scene_settings.h:31-35`
    pub mod atlas {
        pub const HEIGHT: usize = 0;
        pub const WIDTH: usize = 1;
        pub const DEPTH: usize = 2;
        pub const CAMERA: usize = 3;
        pub const CONTOURS: usize = 4;
        pub const COLOR: usize = 5;
        pub const SPEED: usize = 6;
        pub const WIREFRAME: usize = 7;
        pub const DETAIL: usize = 8;
        pub const HUE_MOTION: usize = 9;
        pub const ORBIT: usize = 10;
        pub const ZOOM: usize = 11;
    }
    /// `scene_settings.h:36-39`
    pub mod terrarium {
        pub const MOTION: usize = 0;
        pub const GROWTH: usize = 1;
        pub const PARTICLES: usize = 2;
        pub const SIM_SPEED: usize = 3;
        pub const CREATURE_SPEED: usize = 4;
        pub const GLASS_OPACITY: usize = 5;
        pub const DENSITY: usize = 6;
        pub const CREATURE_GLOW: usize = 7;
    }
    /// `scene_settings.h:40-43`
    pub mod constellation {
        pub const MOTION: usize = 0;
        pub const SCALE: usize = 1;
        pub const GLOW: usize = 2;
        pub const EVENT_DURATION: usize = 3;
        pub const EVENT_REACH: usize = 4;
        pub const HUE_SWING: usize = 5;
        pub const DENSITY: usize = 6;
        pub const WEB: usize = 7;
    }
    /// `scene_settings.h:44-46`
    pub mod cadence {
        pub const SCALE: usize = 0;
        pub const SWARM: usize = 1;
        pub const FOCUS: usize = 2;
        pub const BEAT: usize = 3;
        pub const GLOW: usize = 4;
        pub const SPACING: usize = 5;
        pub const HUE_SWING: usize = 6;
    }
    /// `scene_settings.h:47-49`
    pub mod loom {
        pub const DENSITY: usize = 0;
        pub const WEIGHT: usize = 1;
        pub const COMPLEXITY: usize = 2;
        pub const EDGE: usize = 3;
        pub const SATURATION: usize = 4;
        pub const MOTION: usize = 5;
        pub const GLINTS: usize = 6;
    }
    /// `scene_settings.h:50-53`
    pub mod pentagram {
        pub const MOTION: usize = 0;
        pub const NEST: usize = 1;
        pub const ORBITS: usize = 2;
        pub const GLOW: usize = 3;
        pub const CHORDS: usize = 4;
        pub const HUE: usize = 5;
        pub const PULSE: usize = 6;
        pub const ZOOM: usize = 7;
    }
    /// Phosphor Dream. No C counterpart — this scene is a post-legacy addition
    /// (2026-08-08), so there is no `scene_settings.h` line to cite.
    pub mod phosphor {
        /// `0` runs the sequence; `1..=FIELD_COUNT` pins one field.
        pub const FIELD: usize = 0;
        pub const MOTION: usize = 1;
        pub const BREATHE: usize = 2;
        pub const DENSITY: usize = 3;
        /// `0` lets each field choose its own alphabet; `1..=RAMP_COUNT` pins one.
        pub const RAMP: usize = 4;
        pub const BLOOM: usize = 5;
        pub const ABERRATION: usize = 6;
        pub const SCANLINES: usize = 7;
        pub const HUE: usize = 8;
        pub const REACTIVITY: usize = 9;
        pub const DWELL: usize = 10;
        pub const TITLES: usize = 11;
    }
}

use SettingDescriptor as D;

/// `scene_settings.c:13-22`
#[rustfmt::skip]
const SPECTRUM: &[SettingDescriptor] = &[
    D::slider("settings.spectrum.amplitude", "Amplitude", 0.40, 2.00, 1.00, 2),
    D::slider("settings.spectrum.trail", "Trail size", 0.25, 2.50, 1.00, 2),
    D::slider("settings.spectrum.saturation", "Saturation", 0.25, 1.25, 1.00, 2),
    D::slider("settings.spectrum.glow_softness", "Glow softness", 1.00, 8.00, 3.00, 2),
    D::slider("settings.spectrum.hue_swing", "Semantic hue swing", 0.00, 120.00, 55.00, 0),
    D::slider("settings.spectrum.core_glow", "Core glow size", 0.00, 3.00, 1.00, 2),
    D::slider("settings.spectrum.bar_taper", "Bar taper", 0.30, 1.50, 0.50, 2),
    D::slider("settings.spectrum.reflection", "Floor reflection", 0.00, 1.00, 0.30, 2),
];

/// `scene_settings.c:24-33`
#[rustfmt::skip]
const PULSE: &[SettingDescriptor] = &[
    D::slider("settings.pulse.scale", "Field scale", 0.55, 1.15, 1.00, 2),
    D::slider("settings.pulse.rings", "Ring count", 6.00, 48.00, 24.00, 0),
    D::slider("settings.pulse.motion", "Rotation speed", 0.00, 2.00, 1.00, 2),
    D::slider("settings.pulse.arc", "Arc length", 0.50, 1.50, 1.00, 2),
    D::slider("settings.pulse.weight", "Line weight", 0.30, 2.50, 1.00, 2),
    D::slider("settings.pulse.petals", "Petal fold (0 = auto)", 0.00, 12.00, 0.00, 0),
    D::slider("settings.pulse.hue", "Hue shift (deg)", -180.0, 180.0, 0.0, 0),
    D::slider("settings.pulse.glow", "Center bloom", 0.00, 2.00, 1.00, 2),
];

/// `scene_settings.c:35-45`
#[rustfmt::skip]
const ORBITAL: &[SettingDescriptor] = &[
    D::slider("settings.orbital.motion", "Motion speed", 0.15, 2.00, 1.00, 2),
    D::slider("settings.orbital.radius", "Lattice radius", 0.55, 1.45, 1.00, 2),
    D::slider("settings.orbital.depth", "Depth spacing", 0.55, 1.55, 1.00, 2),
    D::slider("settings.orbital.nodes", "Node size", 0.35, 2.20, 1.00, 2),
    D::slider("settings.orbital.links", "Link weight", 0.00, 2.20, 1.00, 2),
    D::slider("settings.orbital.tilt", "Camera tilt", 0.00, 2.00, 1.00, 2),
    D::slider("settings.orbital.hue", "Hue shift (deg)", -180.0, 180.0, 0.0, 0),
    D::slider("settings.orbital.reactivity", "Beat reactivity", 0.00, 2.00, 1.00, 2),
    D::slider("settings.orbital.sway", "Drift & sway", 0.00, 2.00, 1.00, 2),
];

/// `scene_settings.c:47-54`
#[rustfmt::skip]
const ASCII: &[SettingDescriptor] = &[
    D::slider("settings.ascii.motion", "Wave motion", 0.00, 2.00, 1.00, 2),
    D::slider("settings.ascii.cycling", "Glyph cycling", 0.00, 2.00, 1.00, 2),
    D::slider("settings.ascii.scanlines", "Scanlines", 0.00, 2.00, 1.00, 2),
    D::slider("settings.ascii.split", "Color split", 0.00, 2.00, 1.00, 2),
    D::slider("settings.ascii.gain", "Brightness gain", 0.50, 2.50, 1.30, 2),
    D::slider("settings.ascii.tint", "Band hue spread", 0.00, 1.00, 0.45, 2),
];

/// `scene_settings.c:56-69`
#[rustfmt::skip]
const ATLAS: &[SettingDescriptor] = &[
    D::slider("settings.atlas.height", "Terrain height", 0.35, 2.75, 1.00, 2),
    D::slider("settings.atlas.width", "Terrain width", 0.55, 3.20, 1.00, 2),
    D::slider("settings.atlas.depth", "Depth spacing", 0.50, 1.65, 1.00, 2),
    D::slider("settings.atlas.camera", "Camera height", 0.25, 1.75, 1.00, 2),
    D::slider("settings.atlas.contours", "Contour weight", 0.00, 2.50, 1.00, 2),
    D::slider("settings.atlas.color", "Hue shift (deg)", -180.0, 180.0, 0.0, 0),
    D::slider("settings.atlas.speed", "Camera drift", 0.00, 2.50, 1.00, 2),
    D::toggle("settings.atlas.wireframe", "Surface style", 0.0),
    D::slider("settings.atlas.detail", "Sampling detail", 1.00, 3.00, 1.00, 0),
    D::toggle("settings.atlas.hue_motion", "Hue motion", 0.0),
    D::slider("settings.atlas.orbit", "Camera orbit", -180.0, 180.0, 0.0, 0),
    D::slider("settings.atlas.zoom", "Camera distance", 0.60, 1.80, 1.00, 2),
];

/// `scene_settings.c:71-80`
#[rustfmt::skip]
const TERRARIUM: &[SettingDescriptor] = &[
    D::slider("settings.terrarium.motion", "Camera motion", 0.00, 2.00, 1.00, 2),
    D::slider("settings.terrarium.growth", "Plant growth", 0.40, 1.80, 1.00, 2),
    D::slider("settings.terrarium.particles", "Particle size", 0.00, 2.20, 1.00, 2),
    D::slider("settings.terrarium.sim_speed", "Ecosystem motion", 0.20, 2.50, 1.00, 2),
    D::slider("settings.terrarium.creature_speed", "Creature speed", 0.20, 2.00, 1.00, 2),
    D::slider("settings.terrarium.glass_opacity", "Habitat glass", 0.00, 0.40, 0.13, 2),
    D::slider("settings.terrarium.density", "Population density", 0.30, 1.00, 1.00, 2),
    D::slider("settings.terrarium.creature_glow", "Creature glow", 0.00, 2.00, 1.00, 2),
];

/// `scene_settings.c:82-91`
#[rustfmt::skip]
const CONSTELLATION: &[SettingDescriptor] = &[
    D::slider("settings.constellation.motion", "Camera motion", 0.00, 2.00, 1.00, 2),
    D::slider("settings.constellation.scale", "Field scale", 0.50, 1.60, 1.00, 2),
    D::slider("settings.constellation.glow", "Node glow", 0.00, 2.20, 1.00, 2),
    D::slider("settings.constellation.event_duration", "Event glow duration", 0.50, 5.00, 2.40, 2),
    D::slider("settings.constellation.event_reach", "Event spread", 1.00, 6.00, 2.00, 0),
    D::slider("settings.constellation.hue_swing", "Semantic hue swing", 0.00, 140.00, 70.00, 0),
    D::slider("settings.constellation.density", "Star density", 1.00, 3.00, 3.00, 0),
    D::slider("settings.constellation.web", "Web brightness", 0.00, 2.00, 1.00, 2),
];

/// `scene_settings.c:93-101`
#[rustfmt::skip]
const CADENCE: &[SettingDescriptor] = &[
    D::slider("settings.cadence.scale", "Type scale", 0.55, 1.35, 1.00, 2),
    D::slider("settings.cadence.swarm", "Swarm spread", 0.00, 2.00, 1.00, 2),
    D::slider("settings.cadence.focus", "Focus speed", 0.50, 3.00, 1.00, 2),
    D::slider("settings.cadence.beat", "Beat breathing", 0.00, 2.00, 1.00, 2),
    D::slider("settings.cadence.glow", "Particle glow", 0.00, 2.00, 1.00, 2),
    D::slider("settings.cadence.spacing", "Letter spacing", 0.50, 2.00, 1.00, 2),
    D::slider("settings.cadence.hue_swing", "Semantic hue swing", 0.00, 160.00, 80.00, 0),
];

/// `scene_settings.c:103-111`
#[rustfmt::skip]
const LOOM: &[SettingDescriptor] = &[
    D::slider("settings.loom.density", "Thread density", 0.50, 2.00, 1.00, 2),
    D::slider("settings.loom.weight", "Thread weight", 0.40, 2.50, 1.00, 2),
    D::slider("settings.loom.complexity", "Weave complexity", 0.50, 2.00, 1.00, 2),
    D::slider("settings.loom.edge", "Growth edge", 0.50, 2.00, 1.00, 2),
    D::slider("settings.loom.saturation", "Saturation", 0.30, 1.30, 1.00, 2),
    D::slider("settings.loom.motion", "Beat lift", 0.00, 2.00, 1.00, 2),
    D::slider("settings.loom.glints", "Onset glints", 0.00, 2.00, 1.00, 2),
];

/// `scene_settings.c:113-122`
#[rustfmt::skip]
const PENTAGRAM: &[SettingDescriptor] = &[
    D::slider("settings.pentagram.motion", "Orbit speed", 0.00, 2.50, 1.00, 2),
    D::slider("settings.pentagram.nest", "Nest curves", 2.00, 12.00, 9.00, 0),
    D::slider("settings.pentagram.orbits", "Orbit count", 4.00, 24.00, 14.00, 0),
    D::slider("settings.pentagram.glow", "Spark glow", 0.00, 2.20, 1.00, 2),
    D::slider("settings.pentagram.chords", "Pentagram lines", 0.00, 2.00, 1.00, 2),
    // Note the default: -91 degrees, not 0. It is deliberate in the oracle.
    D::slider("settings.pentagram.hue", "Hue shift (deg)", -180.0, 180.0, -91.0, 0),
    D::slider("settings.pentagram.pulse", "Music coupling", 0.00, 2.00, 1.00, 2),
    D::slider("settings.pentagram.zoom", "Field scale", 0.60, 1.50, 1.00, 2),
];

/// Phosphor Dream, a post-legacy addition with no oracle line to cite.
///
/// Twelve controls, which is exactly [`MAX_CONTROLS`] — Song Atlas set that
/// ceiling and this scene meets it rather than raising it, because the dense
/// value array is `[[f32; MAX_CONTROLS]; SCENE_COUNT]` and widening it grows
/// every scene's row for the benefit of one.
///
/// Two of them are **selectors** rather than magnitudes, and both spell zero as
/// "let the scene decide": `field` 0 runs the whole sequence and 1..=10 pins one
/// field, `ramp` 0 lets each field keep its own alphabet and 1..=7 pins one.
/// Encoding "auto" as a value inside the range rather than as a separate toggle
/// is what keeps them routable and randomizable like anything else — a toggle
/// plus a value would need `tune_explore` to know they are paired.
///
/// `titles` defaults **off**. The source piece surfaces its own words out of the
/// field, and this application already has a caption layer with authored timing;
/// two word layers fighting for the same frame is the wrong default even though
/// the effect is worth keeping.
#[rustfmt::skip]
const PHOSPHOR: &[SettingDescriptor] = &[
    D::slider("settings.phosphor.field", "Field (0 = cycle)", 0.00, 10.00, 0.00, 0),
    D::slider("settings.phosphor.motion", "Field motion", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.breathe", "World drift", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.density", "Cell density", 0.50, 2.00, 1.00, 2),
    D::slider("settings.phosphor.ramp", "Alphabet (0 = auto)", 0.00, 7.00, 0.00, 0),
    D::slider("settings.phosphor.bloom", "CRT bloom", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.aberration", "Colour split", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.scanlines", "Scanlines", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.hue", "Hue shift (deg)", -180.0, 180.0, 0.0, 0),
    D::slider("settings.phosphor.reactivity", "Music coupling", 0.00, 2.00, 1.00, 2),
    D::slider("settings.phosphor.dwell", "Seconds per field", 4.00, 30.00, 10.00, 0),
    D::toggle("settings.phosphor.titles", "Surfacing words", 0.0),
];

/// The descriptor table, in scene registry order (`scene_settings.c:129-141`
/// for the first ten).
const TABLES: [&[SettingDescriptor]; SCENE_COUNT] = [
    SPECTRUM,
    PULSE,
    ORBITAL,
    ASCII,
    ATLAS,
    TERRARIUM,
    CONSTELLATION,
    CADENCE,
    LOOM,
    PENTAGRAM,
    PHOSPHOR,
];

/// Descriptors for one scene.
#[must_use]
pub fn descriptors(scene: SceneId) -> &'static [SettingDescriptor] {
    TABLES[scene.index()]
}

/// Control count for one scene (`scene_settings_count`).
#[must_use]
pub fn count(scene: SceneId) -> usize {
    TABLES[scene.index()].len()
}

/// One descriptor, or `None` when the index is out of range.
#[must_use]
pub fn descriptor(scene: SceneId, index: usize) -> Option<&'static SettingDescriptor> {
    TABLES[scene.index()].get(index)
}

/// Resolves a persisted parameter key like `"settings.loom.weight"`
/// (`scene_settings_descriptor_by_key`).
#[must_use]
pub fn descriptor_by_key(key: &str) -> Option<(SceneId, usize, &'static SettingDescriptor)> {
    for scene in SceneId::ALL {
        for (index, descriptor) in descriptors(scene).iter().enumerate() {
            if descriptor.key == key {
                return Some((scene, index, descriptor));
            }
        }
    }
    None
}

/// Every scene's current values (`scene_settings.h:65-67`).
///
/// A dense `[[f32; MAX_CONTROLS]; SCENE_COUNT]` exactly as in C. Slots past a
/// scene's control count are unused and stay zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneSettings {
    values: [[f32; MAX_CONTROLS]; SCENE_COUNT],
}

impl Default for SceneSettings {
    /// Every control at its descriptor default (`scene_settings_init`).
    fn default() -> Self {
        let mut values = [[0.0f32; MAX_CONTROLS]; SCENE_COUNT];
        for scene in SceneId::ALL {
            for (index, descriptor) in descriptors(scene).iter().enumerate() {
                values[scene.index()][index] = descriptor.default_value;
            }
        }
        Self { values }
    }
}

impl SceneSettings {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True when every value is in range for its descriptor
    /// (`scene_settings_valid`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        SceneId::ALL.into_iter().all(|scene| {
            descriptors(scene)
                .iter()
                .enumerate()
                .all(|(index, descriptor)| descriptor.accepts(self.values[scene.index()][index]))
        })
    }

    /// Reads one value.
    ///
    /// Two fallbacks copied from `scene_settings_get` (`scene_settings.c:187-198`),
    /// both load-bearing:
    /// - an unknown index returns `1.0`, because most settings are multipliers
    ///   and 1.0 is the neutral one;
    /// - a stored value that is out of range returns the *default*, not a clamp.
    #[must_use]
    pub fn get(&self, scene: SceneId, index: usize) -> f32 {
        let Some(descriptor) = descriptor(scene, index) else {
            return 1.0;
        };
        let value = self.values[scene.index()][index];
        if descriptor.accepts(value) {
            value
        } else {
            descriptor.default_value
        }
    }

    /// Writes one value, rejecting anything out of range (`scene_settings_set`).
    ///
    /// Returns `false` and changes nothing on rejection — it does not clamp.
    pub fn set(&mut self, scene: SceneId, index: usize, value: f32) -> bool {
        let Some(descriptor) = descriptor(scene, index) else {
            return false;
        };
        if !descriptor.accepts(value) {
            return false;
        }
        self.values[scene.index()][index] = value;
        true
    }

    /// Restores one scene's defaults (`scene_settings_reset_scene`).
    pub fn reset_scene(&mut self, scene: SceneId) {
        for (index, descriptor) in descriptors(scene).iter().enumerate() {
            self.values[scene.index()][index] = descriptor.default_value;
        }
    }

    /// Captures one scene's values, or `None` if any is out of range
    /// (`scene_settings_capture`).
    #[must_use]
    pub fn capture(&self, scene: SceneId) -> Option<SettingsSnapshot> {
        let mut snapshot = SettingsSnapshot {
            captured: true,
            count: count(scene),
            values: [0.0; MAX_CONTROLS],
        };
        for (index, descriptor) in descriptors(scene).iter().enumerate() {
            let value = self.values[scene.index()][index];
            if !descriptor.accepts(value) {
                return None;
            }
            snapshot.values[index] = value;
        }
        Some(snapshot)
    }

    /// Applies a snapshot, back-filling any control the snapshot predates from
    /// its descriptor default (`scene_settings_apply_snapshot`).
    ///
    /// Returns `false` for a snapshot that is not valid for this scene.
    pub fn apply_snapshot(&mut self, scene: SceneId, snapshot: &SettingsSnapshot) -> bool {
        if !snapshot.is_valid_for(scene) {
            return false;
        }
        if !snapshot.captured {
            return true;
        }
        for (index, descriptor) in descriptors(scene).iter().enumerate() {
            self.values[scene.index()][index] = if index < snapshot.count {
                snapshot.values[index]
            } else {
                descriptor.default_value
            };
        }
        true
    }
}

/// A captured set of one scene's values (`scene_settings_values.h:13-17`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingsSnapshot {
    pub captured: bool,
    pub count: usize,
    pub values: [f32; MAX_CONTROLS],
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            captured: false,
            count: 0,
            values: [0.0; MAX_CONTROLS],
        }
    }
}

/// Control counts a scene used to have, which must stay loadable.
///
/// Copied from `scene_settings_count_is_legacy` (`scene_settings.c:239-251`).
/// This is pure compatibility history: a project saved by an older build carries
/// fewer values, and the missing ones back-fill from defaults. The three scenes
/// added last (cadence, loom, pentagram) have no legacy counts because their
/// control set never shrank.
#[must_use]
pub fn count_is_legacy(scene: SceneId, count: usize) -> bool {
    match scene {
        SceneId::Spectrum => count == 3 || count == 7,
        SceneId::PulseField => count == 5,
        SceneId::OrbitalLattice => count == 5 || count == 7,
        SceneId::AsciiField => count == 4,
        SceneId::SongAtlas => count == 8 || count == 10,
        SceneId::SpectralTerrarium => count == 3 || count == 7,
        SceneId::Constellation => count == 3 || count == 7,
        // Phosphor Dream shipped with twelve and has never had another count;
        // it postdates the C entirely, so there is no legacy shape to admit.
        SceneId::Cadence | SceneId::Loom | SceneId::Pentagram | SceneId::PhosphorDream => false,
    }
}

impl SettingsSnapshot {
    /// `scene_settings_snapshot_valid` (`scene_settings.c:253-...`).
    #[must_use]
    pub fn is_valid_for(&self, scene: SceneId) -> bool {
        if !self.captured {
            return self.count == 0;
        }
        if self.count != count(scene) && !count_is_legacy(scene, self.count) {
            return false;
        }
        if self.count > MAX_CONTROLS {
            return false;
        }
        descriptors(scene)
            .iter()
            .take(self.count)
            .enumerate()
            .all(|(index, descriptor)| descriptor.accepts(self.values[index]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_has_controls_and_none_exceeds_the_cap() {
        for scene in SceneId::ALL {
            let n = count(scene);
            assert!(n > 0, "{scene:?} has no controls");
            assert!(
                n <= MAX_CONTROLS,
                "{scene:?} has {n} controls, cap is {MAX_CONTROLS}"
            );
        }
        // Song Atlas is what sets the cap.
        assert_eq!(count(SceneId::SongAtlas), MAX_CONTROLS);
    }

    #[test]
    fn the_c_control_counts_are_reproduced_exactly() {
        let expected = [8, 8, 9, 6, 12, 8, 8, 7, 7, 8];
        for (scene, want) in SceneId::ALL.into_iter().zip(expected) {
            assert_eq!(count(scene), want, "{scene:?} control count");
        }
    }

    #[test]
    fn defaults_are_in_range_and_every_key_is_unique() {
        let mut keys = std::collections::HashSet::new();
        for scene in SceneId::ALL {
            for descriptor in descriptors(scene) {
                assert!(
                    descriptor.accepts(descriptor.default_value),
                    "{} default {} is outside {}..{}",
                    descriptor.key,
                    descriptor.default_value,
                    descriptor.minimum,
                    descriptor.maximum
                );
                assert!(
                    descriptor.minimum <= descriptor.maximum,
                    "{}",
                    descriptor.key
                );
                assert!(
                    keys.insert(descriptor.key),
                    "duplicate key {}",
                    descriptor.key
                );
                // The persisted key must name its own scene.
                assert!(
                    descriptor.key.starts_with("settings."),
                    "{} is not a settings key",
                    descriptor.key
                );
            }
        }
    }

    #[test]
    fn a_default_settings_value_is_valid() {
        assert!(SceneSettings::default().is_valid());
    }

    #[test]
    fn out_of_range_writes_are_rejected_not_clamped() {
        let mut settings = SceneSettings::default();
        let amplitude = index::spectrum::AMPLITUDE;
        assert!(!settings.set(SceneId::Spectrum, amplitude, 99.0));
        assert!(!settings.set(SceneId::Spectrum, amplitude, 0.0));
        assert!(!settings.set(SceneId::Spectrum, amplitude, f32::NAN));
        assert_eq!(
            settings.get(SceneId::Spectrum, amplitude),
            1.00,
            "value untouched"
        );
        assert!(settings.set(SceneId::Spectrum, amplitude, 1.75));
        assert_eq!(settings.get(SceneId::Spectrum, amplitude), 1.75);
    }

    #[test]
    fn a_toggle_rejects_intermediate_values() {
        let mut settings = SceneSettings::default();
        let wireframe = index::atlas::WIREFRAME;
        assert_eq!(
            descriptor(SceneId::SongAtlas, wireframe).unwrap().kind,
            SettingKind::Toggle
        );
        assert!(!settings.set(SceneId::SongAtlas, wireframe, 0.5));
        assert!(settings.set(SceneId::SongAtlas, wireframe, 1.0));
        assert!(settings.set(SceneId::SongAtlas, wireframe, 0.0));
    }

    #[test]
    fn an_unknown_index_reads_as_the_neutral_multiplier() {
        let settings = SceneSettings::default();
        assert_eq!(settings.get(SceneId::Spectrum, 999), 1.0);
    }

    #[test]
    fn keys_resolve_back_to_their_scene_and_index() {
        let (scene, index, descriptor) = descriptor_by_key("settings.loom.weight").unwrap();
        assert_eq!(scene, SceneId::Loom);
        assert_eq!(index, index::loom::WEIGHT);
        assert_eq!(descriptor.label, "Thread weight");
        assert!(descriptor_by_key("settings.nope.nope").is_none());
    }

    #[test]
    fn pentagram_hue_keeps_its_unusual_default() {
        // -91 degrees is deliberate in the oracle; a "tidied" 0.0 would be a
        // visible parity break.
        assert_eq!(
            descriptor(SceneId::Pentagram, index::pentagram::HUE)
                .unwrap()
                .default_value,
            -91.0
        );
    }

    #[test]
    fn a_snapshot_round_trips() {
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 2.25);
        let snapshot = settings.capture(SceneId::Loom).unwrap();
        let mut restored = SceneSettings::default();
        assert!(restored.apply_snapshot(SceneId::Loom, &snapshot));
        assert_eq!(restored.get(SceneId::Loom, index::loom::WEIGHT), 2.25);
    }

    #[test]
    fn a_legacy_snapshot_back_fills_missing_controls_from_defaults() {
        // A three-value spectrum snapshot is what an early build wrote.
        let mut snapshot = SettingsSnapshot {
            captured: true,
            count: 3,
            values: [0.0; MAX_CONTROLS],
        };
        snapshot.values[0] = 1.5; // amplitude
        snapshot.values[1] = 2.0; // trail
        snapshot.values[2] = 0.8; // saturation
        assert!(snapshot.is_valid_for(SceneId::Spectrum));

        let mut settings = SceneSettings::default();
        assert!(settings.apply_snapshot(SceneId::Spectrum, &snapshot));
        assert_eq!(
            settings.get(SceneId::Spectrum, index::spectrum::AMPLITUDE),
            1.5
        );
        assert_eq!(settings.get(SceneId::Spectrum, index::spectrum::TRAIL), 2.0);
        // Reflection did not exist then, so it takes the shipped default.
        assert_eq!(
            settings.get(SceneId::Spectrum, index::spectrum::REFLECTION),
            0.30
        );
    }

    #[test]
    fn a_snapshot_of_the_wrong_size_is_rejected() {
        let snapshot = SettingsSnapshot {
            captured: true,
            count: 6,
            values: [1.0; MAX_CONTROLS],
        };
        // Spectrum's legacy counts are 3 and 7; 6 was never a spectrum size.
        assert!(!snapshot.is_valid_for(SceneId::Spectrum));
        // But 6 is ASCII Field's current count.
        let ascii = SettingsSnapshot {
            captured: true,
            count: 6,
            values: [1.0; MAX_CONTROLS],
        };
        assert!(ascii.is_valid_for(SceneId::AsciiField));
    }

    #[test]
    fn an_uncaptured_snapshot_is_only_valid_when_empty() {
        let empty = SettingsSnapshot::default();
        assert!(empty.is_valid_for(SceneId::Cadence));
        let bogus = SettingsSnapshot {
            captured: false,
            count: 4,
            values: [0.0; MAX_CONTROLS],
        };
        assert!(!bogus.is_valid_for(SceneId::Cadence));
    }
}
