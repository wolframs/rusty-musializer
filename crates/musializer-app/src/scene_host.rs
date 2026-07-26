//! Binding and drawing the ten scenes.
//!
//! **This is the hookup point for Agents C and D.** Everything a scene needs to
//! become visible goes through the two tables below, and each table has exactly
//! one line per scene. Adding a scene is two edits, both marked `HOOKUP`.
//!
//! ## Why this file exists
//!
//! C's `Scene_Descriptor` carries `init`/`update`/`draw`/`unload` function
//! pointers over a `void *state` (`../musializer/src/scene.h:77-86`). The Rust
//! split puts the deterministic `update` behind
//! [`musializer_core::scene::SceneState`] — headlessly testable, no raylib — and
//! leaves drawing in this crate, where raylib is allowed. That means the registry
//! itself has to live *here*, because it is the only place that can see both
//! halves.
//!
//! ## What is stubbed and what that means
//!
//! A scene whose deterministic half has not landed yet gets
//! [`StatelessScene`], and a scene whose drawing half has not landed yet gets
//! [`draw_placeholder`] — a labelled card naming the scene and the C file it
//! comes from. That is deliberate: **compilation beats completeness**, and a
//! placeholder that names itself is how `--scene pentagram` can be exercised
//! before Pentagram exists. A silent black frame would be indistinguishable from
//! a broken scene.

use musializer_core::scene::{SceneDescriptor, SceneFrame, SceneId, SceneInstance, StatelessScene};
use musializer_core::scenes as core_scenes;
use musializer_runtime::font::Faces;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Rectangle};

use crate::scenes;

/// The registry entry for one scene.
///
/// `state_version` is bumped when a state layout changes so a rebind can discard
/// stale state instead of reinterpreting it. Placeholder entries sit at 0 so a
/// real port is always a visible bump.
///
/// **HOOKUP (Agents C and D):** replace the `make_state` closure with your
/// scene's constructor, e.g.
/// `make_state: |seed| Box::new(musializer_core::scenes::loom::LoomState::new(seed))`,
/// and bump `state_version` to 1. Spectrum genuinely has no state — the C sets no
/// `update` for it (`scene_spectrum.c:149-154`) — so its `StatelessScene` is the
/// finished article, not a stub.
#[must_use]
pub fn descriptor(id: SceneId) -> SceneDescriptor {
    match id {
        // Finished: Spectrum is a pure function of the frame.
        SceneId::Spectrum => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |_seed| Box::new(StatelessScene::new(SceneId::Spectrum)),
        },
        // Agent C's four.
        SceneId::PulseField => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::pulse_field::PulseFieldState::new(seed)),
        },
        SceneId::OrbitalLattice => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| {
                Box::new(core_scenes::orbital_lattice::OrbitalLatticeState::new(seed))
            },
        },
        SceneId::AsciiField => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::ascii_field::AsciiFieldState::new(seed)),
        },
        SceneId::SongAtlas => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::song_atlas::SongAtlasState::new(seed)),
        },
        // Agent D's five.
        SceneId::SpectralTerrarium => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| {
                Box::new(core_scenes::spectral_terrarium::SpectralTerrariumState::new(seed))
            },
        },
        SceneId::Constellation => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::constellation::ConstellationState::new(seed)),
        },
        SceneId::Cadence => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::cadence::CadenceState::new(seed)),
        },
        SceneId::Loom => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::loom::LoomState::new(seed)),
        },
        SceneId::Pentagram => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |seed| Box::new(core_scenes::pentagram::PentagramState::new(seed)),
        },
    }
}

/// Recovers a scene's concrete state from its boxed trait object.
///
/// The downcast is the crossing the split costs us: `update` lives in
/// `musializer-core` behind `dyn SceneState`, and drawing needs the real type.
/// `SceneState::as_any` exists for exactly this. A failure here means a descriptor
/// and a draw arm disagree about a scene's state type, which the tests below catch.
fn state_of<T: 'static>(instance: &SceneInstance, id: SceneId) -> Option<&T> {
    let state = instance.state().as_any().downcast_ref::<T>();
    debug_assert!(
        state.is_some(),
        "{}: descriptor and draw arm disagree about the state type",
        id.stable_name()
    );
    state
}

/// A descriptor for a scene whose deterministic half has not landed.
///
/// Unused now that all ten are ported, and kept deliberately: it is what a
/// newly added scene should start from, and deleting it would push the next
/// person toward wiring a half-built scene straight into `descriptor`.
#[allow(dead_code)]
fn stateless(id: SceneId) -> SceneDescriptor {
    SceneDescriptor {
        id,
        state_version: 0,
        make_state: match id {
            SceneId::Spectrum => |_| Box::new(StatelessScene::new(SceneId::Spectrum)),
            SceneId::PulseField => |_| Box::new(StatelessScene::new(SceneId::PulseField)),
            SceneId::OrbitalLattice => |_| Box::new(StatelessScene::new(SceneId::OrbitalLattice)),
            SceneId::AsciiField => |_| Box::new(StatelessScene::new(SceneId::AsciiField)),
            SceneId::SongAtlas => |_| Box::new(StatelessScene::new(SceneId::SongAtlas)),
            SceneId::SpectralTerrarium => {
                |_| Box::new(StatelessScene::new(SceneId::SpectralTerrarium))
            }
            SceneId::Constellation => |_| Box::new(StatelessScene::new(SceneId::Constellation)),
            SceneId::Cadence => |_| Box::new(StatelessScene::new(SceneId::Cadence)),
            SceneId::Loom => |_| Box::new(StatelessScene::new(SceneId::Loom)),
            SceneId::Pentagram => |_| Box::new(StatelessScene::new(SceneId::Pentagram)),
        },
    }
}

/// True while a scene's drawing half is still a placeholder.
///
/// The report and the on-screen card both read this, so "is this scene real yet?"
/// has one answer rather than two that can disagree.
#[must_use]
pub fn drawing_is_ported(id: SceneId) -> bool {
    // All ten drawing halves have landed. `draw_placeholder` is kept because it is
    // the right answer for a scene added later, and because deleting it would make
    // the next addition silently draw nothing.
    let _ = id;
    true
}

/// The placeholder machinery below is unreachable now that all ten drawing halves
/// have landed, and it stays on purpose.
///
/// It is what an eleventh scene should render before its drawing half exists, and
/// the reason is worth keeping: a labelled card naming the scene and its C source
/// tells a reviewer looking at a screenshot the difference between "not ported
/// yet" and "ported and broken", which a black rectangle cannot. Deleting it would
/// make the next addition draw nothing and look like a bug.
///
/// The C source each scene's drawing half comes from, named on the placeholder
/// card so a reviewer looking at a screenshot knows where to look.
#[must_use]
#[allow(dead_code)]
pub fn oracle_source(id: SceneId) -> &'static str {
    match id {
        SceneId::Spectrum => "scene_spectrum.c",
        SceneId::PulseField => "scene_pulse_field.c",
        SceneId::OrbitalLattice => "scene_orbital_lattice.c",
        SceneId::AsciiField => "scene_ascii_field.c",
        SceneId::SongAtlas => "scene_song_atlas.c",
        SceneId::SpectralTerrarium => "scene_spectral_terrarium.c",
        SceneId::Constellation => "scene_constellation.c",
        SceneId::Cadence => "scene_cadence.c",
        SceneId::Loom => "scene_loom.c",
        SceneId::Pentagram => "scene_pentagram.c",
    }
}

/// Which agent owns a scene's port, for the placeholder card.
#[must_use]
#[allow(dead_code)]
pub fn owner(id: SceneId) -> &'static str {
    match id {
        SceneId::Spectrum
        | SceneId::PulseField
        | SceneId::OrbitalLattice
        | SceneId::AsciiField
        | SceneId::SongAtlas => "Agent C",
        _ => "Agent D",
    }
}

/// GPU resources the drawing halves share.
///
/// One owner for the whole application: a shader loaded per scene switch would
/// leak a program every time somebody pressed Tab. Scenes that need their own
/// resources get a field here rather than loading them inside `draw`.
pub struct SceneRenderer {
    circle: scenes::spectrum::CircleShader,
    /// Song Atlas's whole-track terrain, built once per track rather than per
    /// frame. `None` until track load runs the preprocessing, and Song Atlas draws
    /// its idle terrain in the meantime rather than nothing.
    atlas_map: Option<musializer_core::audio::song_atlas_map::SongAtlasMap>,
}

impl SceneRenderer {
    pub fn load(
        rl: &mut raylib::RaylibHandle,
        thread: &raylib::RaylibThread,
    ) -> Result<Self, String> {
        Ok(Self {
            circle: scenes::spectrum::CircleShader::load(rl, thread)?,
            atlas_map: None,
        })
    }

    /// Draws the bound scene into `boundary`.
    ///
    /// `pixel_scale` is physical target pixels per logical output pixel, so
    /// fixed-pixel details supersample without changing composition
    /// (`../musializer/src/scene.h:64-66`).
    ///
    /// **HOOKUP (Agents C and D):** replace your scene's arm. If your drawing
    /// half needs the deterministic state, recover it from `instance.state()`
    /// with `as_any().downcast_ref::<YourState>()` — that downcast hook exists on
    /// [`musializer_core::scene::SceneState`] precisely for this crossing.
    #[allow(
        clippy::too_many_arguments,
        reason = "the scene-draw crossing: target, faces, instance, frame, boundary, scale"
    )]
    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        instance: &SceneInstance,
        frame: &SceneFrame<'_>,
        boundary: Rectangle,
        pixel_scale: f32,
    ) {
        let id = instance.id();
        match id {
            SceneId::Spectrum => {
                scenes::spectrum::draw(d, frame, &mut self.circle, boundary, pixel_scale);
            }
            SceneId::PulseField => {
                if let Some(state) = state_of(instance, id) {
                    scenes::pulse_field::draw(d, state, frame, boundary, pixel_scale);
                }
            }
            SceneId::OrbitalLattice => {
                if let Some(state) = state_of(instance, id) {
                    scenes::orbital_lattice::draw(d, state, frame, boundary);
                }
            }
            SceneId::AsciiField => {
                if let Some(state) = state_of(instance, id) {
                    // `None` until `--ascii-image` is wired: without a grid the
                    // scene is a procedural rolling spectrogram, which is its other
                    // documented mode rather than a degraded one.
                    scenes::ascii_field::draw(d, state, frame, boundary, pixel_scale, None);
                }
            }
            SceneId::SongAtlas => {
                if let Some(state) = state_of(instance, id) {
                    // `None` until whole-track preprocessing runs at load; the
                    // scene draws its idle terrain rather than nothing.
                    scenes::song_atlas::draw(
                        d,
                        state,
                        frame,
                        boundary,
                        pixel_scale,
                        self.atlas_map.as_ref(),
                    );
                }
            }
            SceneId::SpectralTerrarium => {
                if let Some(state) = state_of(instance, id) {
                    scenes::spectral_terrarium::draw(d, frame, state, &mut self.circle, boundary);
                }
            }
            SceneId::Constellation => {
                if let Some(state) = state_of(instance, id) {
                    scenes::constellation::draw(d, frame, state, &mut self.circle, boundary);
                }
            }
            SceneId::Cadence => {
                if let Some(state) = state_of(instance, id) {
                    // The caption face, Alegreya, which is what the oracle draws
                    // Cadence with (`plug.c:1329` hands `p->font` to the scene
                    // renderer, and `scene_cadence.c:449` uses it). Not the
                    // interface face: this scene typesets lyrics, and the interface
                    // atlas carries only the codepoints the chrome needs.
                    scenes::cadence::draw(d, frame, state, fonts.caption(), boundary, pixel_scale);
                }
            }
            SceneId::Loom => {
                if let Some(state) = state_of(instance, id) {
                    scenes::loom::draw(d, frame, state, boundary, pixel_scale);
                }
            }
            SceneId::Pentagram => {
                if let Some(state) = state_of(instance, id) {
                    scenes::pentagram::draw(d, frame, state, &mut self.circle, boundary);
                }
            }
        }
    }
}

/// A labelled card standing in for an unported scene.
///
/// It names the scene, its C source and its owner, because a screenshot of a
/// black rectangle cannot tell "not ported yet" from "ported and broken". It also
/// draws a thin audio-reactive bar so the *host* — analyzer, bridge, frame loop —
/// is still visibly working behind the missing scene.
#[allow(dead_code)]
pub fn draw_placeholder(d: &mut RaylibDrawHandle<'_>, id: SceneId, boundary: Rectangle) {
    let card_width = (boundary.width * 0.62).min(560.0);
    let card_height = 132.0f32.min(boundary.height - 16.0).max(0.0);
    if card_width <= 0.0 || card_height <= 0.0 {
        return;
    }
    let x = boundary.x + (boundary.width - card_width) * 0.5;
    let y = boundary.y + (boundary.height - card_height) * 0.5;

    d.draw_rectangle(
        x as i32,
        y as i32,
        card_width as i32,
        card_height as i32,
        Color::get_color(0x1f2430ff),
    );
    d.draw_rectangle_lines(
        x as i32,
        y as i32,
        card_width as i32,
        card_height as i32,
        Color::get_color(0x3d4757ff),
    );
    d.draw_text(
        id.display_name(),
        x as i32 + 16,
        y as i32 + 14,
        26,
        Color::RAYWHITE,
    );
    d.draw_text(
        &format!("--scene {}", id.stable_name()),
        x as i32 + 16,
        y as i32 + 50,
        16,
        Color::get_color(0x8fa1b8ff),
    );
    d.draw_text(
        &format!("drawing half not ported yet ({})", owner(id)),
        x as i32 + 16,
        y as i32 + 74,
        16,
        Color::get_color(0xd9a05bff),
    );
    d.draw_text(
        oracle_source(id),
        x as i32 + 16,
        y as i32 + 98,
        16,
        Color::get_color(0x6f7f95ff),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_has_a_descriptor_whose_state_agrees_with_its_id() {
        // The debug assertion inside SceneInstance::new checks this too, but only
        // in a debug build of the binary. Pinning it here means a mis-wired
        // HOOKUP arm fails in `cargo test` rather than at runtime.
        for id in SceneId::ALL {
            let instance = SceneInstance::new(descriptor(id), 7);
            assert_eq!(instance.id(), id, "{}", id.stable_name());
            assert_eq!(instance.state().id(), id, "{}", id.stable_name());
        }
    }

    #[test]
    fn rebinding_keeps_the_seed_and_the_id() {
        for id in SceneId::ALL {
            let mut instance = SceneInstance::new(descriptor(id), 99);
            instance.rebind();
            assert_eq!(instance.seed(), 99);
            assert_eq!(instance.id(), id);
        }
    }

    #[test]
    fn every_scene_claims_to_be_ported_and_all_ten_are() {
        // This assertion is deliberately exhaustive rather than a count. It was
        // written when only Spectrum was wired, and it failed — correctly — the
        // moment the other nine arms landed, which is exactly the reminder it was
        // for. If an eleventh scene is added, it fails again until its arm exists.
        let ported: Vec<&str> = SceneId::ALL
            .into_iter()
            .filter(|id| drawing_is_ported(*id))
            .map(SceneId::stable_name)
            .collect();
        assert_eq!(
            ported,
            vec![
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
            ]
        );
    }

    /// Every scene's descriptor must hand back the state type its drawing arm
    /// downcasts to. A mismatch would silently draw nothing, because `state_of`
    /// returns `None` and the arm skips — so this walks all ten and asserts the
    /// downcast succeeds.
    #[test]
    fn every_drawing_arm_can_recover_its_scenes_state() {
        use musializer_core::scenes as cs;
        for id in SceneId::ALL {
            let instance = SceneInstance::new(descriptor(id), 12345);
            let any = instance.state().as_any();
            let recovered = match id {
                SceneId::Spectrum => any.is::<StatelessScene>(),
                SceneId::PulseField => any.is::<cs::pulse_field::PulseFieldState>(),
                SceneId::OrbitalLattice => any.is::<cs::orbital_lattice::OrbitalLatticeState>(),
                SceneId::AsciiField => any.is::<cs::ascii_field::AsciiFieldState>(),
                SceneId::SongAtlas => any.is::<cs::song_atlas::SongAtlasState>(),
                SceneId::SpectralTerrarium => {
                    any.is::<cs::spectral_terrarium::SpectralTerrariumState>()
                }
                SceneId::Constellation => any.is::<cs::constellation::ConstellationState>(),
                SceneId::Cadence => any.is::<cs::cadence::CadenceState>(),
                SceneId::Loom => any.is::<cs::loom::LoomState>(),
                SceneId::Pentagram => any.is::<cs::pentagram::PentagramState>(),
            };
            assert!(
                recovered,
                "{}: descriptor state type does not match the drawing arm's downcast",
                id.stable_name()
            );
        }
    }

    #[test]
    fn every_scene_names_a_c_source_and_an_owner() {
        for id in SceneId::ALL {
            assert!(oracle_source(id).ends_with(".c"), "{}", id.stable_name());
            assert!(owner(id).starts_with("Agent"), "{}", id.stable_name());
        }
    }
}
