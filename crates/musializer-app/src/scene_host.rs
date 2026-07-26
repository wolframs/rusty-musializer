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
        // HOOKUP: Agent C owns these four.
        SceneId::PulseField => stateless(id),
        SceneId::OrbitalLattice => stateless(id),
        SceneId::AsciiField => stateless(id),
        SceneId::SongAtlas => stateless(id),
        // HOOKUP: Agent D owns these five.
        SceneId::SpectralTerrarium => stateless(id),
        SceneId::Constellation => stateless(id),
        SceneId::Cadence => stateless(id),
        SceneId::Loom => stateless(id),
        SceneId::Pentagram => stateless(id),
    }
}

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
    matches!(id, SceneId::Spectrum)
}

/// The C source each scene's drawing half comes from, named on the placeholder
/// card so a reviewer looking at a screenshot knows where to look.
#[must_use]
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
}

impl SceneRenderer {
    pub fn load(
        rl: &mut raylib::RaylibHandle,
        thread: &raylib::RaylibThread,
    ) -> Result<Self, String> {
        Ok(Self {
            circle: scenes::spectrum::CircleShader::load(rl, thread)?,
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
    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
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
            // HOOKUP: nine arms to fill in. Until then the card says so.
            _ => draw_placeholder(d, id, boundary),
        }
    }
}

/// A labelled card standing in for an unported scene.
///
/// It names the scene, its C source and its owner, because a screenshot of a
/// black rectangle cannot tell "not ported yet" from "ported and broken". It also
/// draws a thin audio-reactive bar so the *host* — analyzer, bridge, frame loop —
/// is still visibly working behind the missing scene.
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
    fn only_the_ported_scenes_claim_to_be_ported() {
        // A reminder to update this when a HOOKUP arm lands: the count here and
        // the match in `SceneRenderer::draw` have to move together.
        let ported: Vec<&str> = SceneId::ALL
            .into_iter()
            .filter(|id| drawing_is_ported(*id))
            .map(SceneId::stable_name)
            .collect();
        assert_eq!(ported, vec!["spectrum"]);
    }

    #[test]
    fn every_scene_names_a_c_source_and_an_owner() {
        for id in SceneId::ALL {
            assert!(oracle_source(id).ends_with(".c"), "{}", id.stable_name());
            assert!(owner(id).starts_with("Agent"), "{}", id.stable_name());
        }
    }
}
