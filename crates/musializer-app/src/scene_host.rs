//! Binding and drawing the ten scenes.
//!
//! This is the registry and application-overlay crossing. Everything a scene
//! needs to become visible goes through the descriptor/draw tables below, and
//! the project lyric overlay is composed once after the selected draw arm.
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
//! ## Future scene additions
//!
//! All ten current scenes are complete. A future scene whose deterministic half
//! has not landed yet starts with
//! [`StatelessScene`], and a scene whose drawing half has not landed yet gets
//! [`draw_placeholder`] — a labelled card naming the scene and the C file it
//! comes from. That is deliberate: **compilation beats completeness**, and a
//! placeholder that names itself is how `--scene pentagram` can be exercised
//! before Pentagram exists. A silent black frame would be indistinguishable from
//! a broken scene.

use musializer_core::project::caption_effects::EffectInputs;
use musializer_core::scene::{SceneDescriptor, SceneFrame, SceneId, SceneInstance, StatelessScene};
use musializer_core::scenes as core_scenes;
use musializer_runtime::font::Faces;
use musializer_runtime::halo::HaloBlur;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, RaylibScissorModeExt, Rectangle};

use crate::scenes;

/// The registry entry for one scene.
///
/// `state_version` is bumped when a state layout changes so a rebind can discard
/// stale state instead of reinterpreting it. Placeholder entries sit at 0 so a
/// real port is always a visible bump.
///
/// A future scene replaces its initial stateless closure with its state
/// constructor and bumps `state_version`. Spectrum genuinely has no state — the
/// C sets no `update` for it (`scene_spectrum.c:149-154`) — so its
/// `StatelessScene` is the finished article, not a stub.
#[must_use]
pub fn descriptor(id: SceneId) -> SceneDescriptor {
    match id {
        // Finished: Spectrum is a pure function of the frame.
        SceneId::Spectrum => SceneDescriptor {
            id,
            state_version: 1,
            make_state: |_seed| Box::new(StatelessScene::new(SceneId::Spectrum)),
        },
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
        SceneId::PhosphorDream => core_scenes::phosphor_dream::descriptor(),
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
            SceneId::PhosphorDream => |_| Box::new(StatelessScene::new(SceneId::PhosphorDream)),
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
        // The one scene with no oracle behind it. Named rather than left to fall
        // through to a plausible-looking C filename that does not exist.
        SceneId::PhosphorDream => "(no oracle: post-legacy addition)",
    }
}

/// GPU resources the drawing halves share.
///
/// One owner for the whole application: a shader loaded per scene switch would
/// leak a program every time somebody pressed Tab. Scenes that need their own
/// resources get a field here rather than loading them inside `draw`.
/// Whole-track data a scene draws but the renderer does not own.
///
/// Both of these belong to the **track** in the oracle (`track->song_atlas_map`,
/// `track->ascii_cells`), and that is not incidental. A renderer-owned atlas map
/// would keep track A's terrain under track B after a switch, and a
/// renderer-owned glyph grid would do the same with an imported image — a parity
/// bug visible on screen, and one the type system can rule out instead by making
/// these borrowed for the duration of a single frame.
#[derive(Clone, Copy, Default)]
pub struct TrackAssets<'a> {
    /// `renderer->song_atlas_map` (`scene.h:62`). `None` before the whole-track
    /// preprocessing has run, and Song Atlas draws its live idle terrain then
    /// rather than nothing.
    pub atlas_map: Option<&'a musializer_core::audio::song_atlas_map::SongAtlasMap>,
    /// `renderer->ascii_cells` (`scene.h:59-61`). `None` until `--ascii-image` or
    /// the project's bundled image has been decoded, and ASCII Field draws its
    /// procedural rolling spectrogram then — its other documented mode rather than
    /// a degraded one.
    pub ascii_grid: Option<&'a musializer_core::scenes::ascii_field::ascii_art::Grid>,
    /// Project caption typography for the shared application overlay. `None`
    /// only when there is no track, in which case there is no lyric either.
    pub caption_style: Option<&'a musializer_core::project::model::CaptionStyle>,
}

pub struct SceneRenderer {
    circle: scenes::spectrum::CircleShader,
    /// The signed-distance-field text shader, for Cadence.
    ///
    /// `None` when it would not compile, and that is a *supported* state rather
    /// than a startup failure: without it the SDF atlas must not be drawn at all
    /// — a distance field through the normal path is grey blobs — so Cadence
    /// falls back to the bitmap atlas it used before, and [`SceneRenderer::describe`]
    /// says which one a capture is looking at.
    sdf_text: Option<raylib::shaders::Shader>,
    /// Whether the shader was refused, kept apart from `None`-because-unused so
    /// the report can tell "did not compile" from "never asked for".
    sdf_error: Option<String>,
    /// Which face typeset the last Cadence frame: `sdf`, `bitmap`, or `none`
    /// before Cadence has drawn at all. A compiled shader does not prove the
    /// atlas built, so the report carries the outcome and not the intent.
    cadence_text: &'static str,
    /// The offscreen blur behind the caption glow halo (UX0-C11). Owned here
    /// like the other GPU resources: one shader and one buffer pair for the
    /// whole application, not a reload per scene switch.
    caption_halo: HaloBlur,
    /// Phosphor Dream's grid buffer and its own blur.
    ///
    /// A *second* `HaloBlur` rather than sharing `caption_halo`, because the two
    /// want very different buffer sizes on the same frame — a caption's halo is
    /// sized from its glyph box and the scene's from the whole panel — and one
    /// instance would reallocate both every frame. Owned here like every other
    /// GPU resource, so a scene switch does not leak a shader.
    phosphor: scenes::phosphor_dream::PhosphorResources,
    /// What the glow did on the last caption frame — `off`, `blurred`, or
    /// `unavailable` — because a frame with no halo and a frame whose halo
    /// silently failed to build are the same picture.
    caption_halo_last: &'static str,
}

impl SceneRenderer {
    /// The SDF fragment shader, embedded like every other first-party shader.
    ///
    /// GLSL 330 only, matching `CircleShader`: the C hard-codes `GLSL_VERSION`
    /// to 330 and its `glsl120` directory is unreachable there, so a second
    /// variant here would be a file nothing compiles.
    const SDF_TEXT_SOURCE: &'static str =
        include_str!("../../../resources/shaders/glsl330/sdf_text.fs");

    pub fn load(
        rl: &mut raylib::RaylibHandle,
        thread: &raylib::RaylibThread,
    ) -> Result<Self, String> {
        // raylib answers a failed compile with the *default* shader rather than
        // an error, so "did it compile" is a question about the returned id, not
        // about a `Result`. `is_shader_valid` is that check.
        let shader = rl.load_shader_from_memory(thread, None, Some(Self::SDF_TEXT_SOURCE));
        let (sdf_text, sdf_error) = if shader.is_shader_valid() {
            (Some(shader), None)
        } else {
            (
                None,
                Some("the driver refused the GLSL 330 SDF text shader".to_string()),
            )
        };
        Ok(Self {
            circle: scenes::spectrum::CircleShader::load(rl, thread)?,
            sdf_text,
            sdf_error,
            cadence_text: "none",
            caption_halo: HaloBlur::load(rl, thread),
            caption_halo_last: "none",
            phosphor: scenes::phosphor_dream::PhosphorResources::load(rl, thread),
        })
    }

    /// One line naming the scene-text path, for the slice report.
    ///
    /// Evidence rather than assertion, and it is the only place the SDF fallback
    /// is visible: a Cadence capture drawn from the bitmap atlas and one drawn
    /// from the distance field are both plausible pictures of Cadence.
    #[must_use]
    pub fn describe(&self) -> String {
        let shader = match &self.sdf_text {
            Some(_) => "sdf-shader=compiled".to_string(),
            None => format!(
                "sdf-shader=FALLBACK ({})",
                self.sdf_error.as_deref().unwrap_or("absent")
            ),
        };
        format!("{shader}, cadence-text={}", self.cadence_text)
    }

    /// One line naming what Phosphor Dream last drew: its field, its alphabet,
    /// its grid and its bloom.
    ///
    /// Every one of those is invisible in a capture. Two fields in the same
    /// alphabet photograph as the same kind of picture; a grid that collapsed to
    /// its floor still draws something plausible; and a bloom that failed to
    /// build looks exactly like `bloom` turned down. This is a scene that can
    /// draw a good-looking frame in several wrong states, so it says which one.
    #[must_use]
    pub fn describe_phosphor(&self) -> String {
        self.phosphor.describe()
    }

    /// One line naming the caption glow's mechanism and last outcome, for its
    /// own report row rather than a suffix on [`SceneRenderer::describe`]: the
    /// headless gate pins that line's grammar exactly (one assertion is a full
    /// string match), so the halo's evidence lives on a line it owns.
    #[must_use]
    pub fn describe_caption_halo(&self) -> String {
        if self.caption_halo.available() {
            // `describe()` is "rt-blur" when whole, and names a refused mask
            // shader otherwise — a state where the glow still blurs but the
            // soft shadow has fallen back to its hard copy, which no capture
            // can tell from an authored hard shadow.
            format!(
                "{} last={}",
                self.caption_halo.describe(),
                self.caption_halo_last
            )
        } else {
            format!("FALLBACK ({})", self.caption_halo.describe())
        }
    }

    /// Draws the bound scene into `boundary`.
    ///
    /// `pixel_scale` is physical target pixels per logical output pixel, so
    /// fixed-pixel details supersample without changing composition
    /// (`../musializer/src/scene.h:64-66`).
    ///
    /// A future drawing half recovers deterministic state from `instance.state()`
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
        assets: TrackAssets<'_>,
        boundary: Rectangle,
        pixel_scale: f32,
    ) {
        let id = instance.id();
        // Phosphor Dream evaluates its grid and builds its bloom *before* the
        // scene clip opens, and that ordering is load-bearing rather than
        // stylistic: `HaloBlur::render` redirects the framebuffer, and a scissor
        // rect is global GL state in framebuffer coordinates, so an active clip
        // meant for the preview panel would crop the offscreen blur passes. The
        // caption halo below is outside the same block for the same reason.
        if id == SceneId::PhosphorDream {
            if let Some(state) = state_of(instance, id) {
                scenes::phosphor_dream::prepare(d, &mut self.phosphor, state, frame, boundary);
            }
        }
        // The scene clip lives here rather than at the call site, because the
        // caption halo below must run with *no* scissor active: its offscreen
        // blur passes redirect the framebuffer, and a scissor rect is global GL
        // state in screen coordinates that would clip the buffer with a
        // rectangle meant for the preview panel. The rect is the same one the
        // preview loop used to hold around this whole call, so a scene that
        // draws past its boundary still cannot paint over a panel
        // (`plug.c:7712-7716`).
        {
            let mut scene_clip = d.begin_scissor_mode(
                boundary.x as i32,
                boundary.y as i32,
                boundary.width as i32,
                boundary.height as i32,
            );
            let d: &mut RaylibDrawHandle<'_> = &mut scene_clip;
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
                        scenes::ascii_field::draw(
                            d,
                            state,
                            frame,
                            boundary,
                            pixel_scale,
                            assets.ascii_grid,
                        );
                    }
                }
                SceneId::SongAtlas => {
                    if let Some(state) = state_of(instance, id) {
                        scenes::song_atlas::draw(
                            d,
                            state,
                            frame,
                            boundary,
                            pixel_scale,
                            assets.atlas_map,
                        );
                    }
                }
                SceneId::SpectralTerrarium => {
                    if let Some(state) = state_of(instance, id) {
                        scenes::spectral_terrarium::draw(
                            d,
                            frame,
                            state,
                            &mut self.circle,
                            boundary,
                        );
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
                        let bitmap = fonts.caption();
                        // The distance field, but only with the shader that reads it,
                        // and only over the codepoints this cue actually needs — the
                        // atlas is built on the first Cadence frame rather than at
                        // startup, so nine other scenes do not pay for it.
                        //
                        // No cue means the scene draws its idle particle ring and no
                        // type at all, so the atlas is not requested then either: a
                        // 200 ms build for a frame with no glyphs in it is a stall
                        // paid for nothing.
                        let cue = frame
                            .lyric
                            .map(|lyric| lyric.text.as_str())
                            .filter(|text| !text.is_empty());
                        let sdf_face = match (self.sdf_text.is_some(), cue) {
                            (true, Some(text)) => fonts.cadence_sdf(text),
                            _ => None,
                        };
                        let mut faces = match &sdf_face {
                            Some(face) => scenes::cadence::CadenceType {
                                text: face.face(),
                                ink: bitmap,
                                sdf: self.sdf_text.as_mut(),
                            },
                            None => scenes::cadence::CadenceType {
                                text: bitmap,
                                ink: bitmap,
                                sdf: None,
                            },
                        };
                        // `ambient` rather than `bitmap` when there is no cue, because
                        // the two are different frames: one fell back, the other had
                        // nothing to typeset.
                        self.cadence_text = if cue.is_none() {
                            "ambient"
                        } else {
                            faces.describe()
                        };
                        scenes::cadence::draw(d, frame, state, &mut faces, boundary, pixel_scale);
                    }
                }
                SceneId::Loom => {
                    if let Some(state) = state_of(instance, id) {
                        scenes::loom::draw(d, frame, state, boundary, pixel_scale);
                    }
                }
                SceneId::PhosphorDream => {
                    if let Some(state) = state_of(instance, id) {
                        scenes::phosphor_dream::draw(
                            d,
                            &mut self.phosphor,
                            state,
                            frame,
                            boundary,
                            pixel_scale,
                        );
                    }
                }
                SceneId::Pentagram => {
                    if let Some(state) = state_of(instance, id) {
                        scenes::pentagram::draw(d, frame, state, &mut self.circle, boundary);
                    }
                }
            }
        }
        if id != SceneId::Cadence {
            if let Some(style) = assets.caption_style {
                // The effect drives read the same frame the scene just drew
                // from, which is what makes an export's glow pulse identical to
                // the preview's.
                let inputs = EffectInputs::from_audio(
                    frame.time_seconds,
                    frame.audio.rms,
                    frame.audio.trails,
                    frame.audio.beat_phase,
                    frame.audio.spectral_flux,
                );
                self.caption_halo_last = scenes::caption::draw_scene_lyric_overlay(
                    d,
                    frame.lyric,
                    fonts,
                    boundary,
                    pixel_scale,
                    style,
                    &inputs,
                    &mut self.caption_halo,
                );
            }
        }
    }
}

/// A labelled card standing in for an unported scene.
///
/// It names the scene and its C source, because a screenshot of a
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
        "drawing half not ported yet",
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
        // registry arm fails in `cargo test` rather than at runtime.
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
                "phosphor",
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
                SceneId::PhosphorDream => any.is::<cs::phosphor_dream::PhosphorDreamState>(),
            };
            assert!(
                recovered,
                "{}: descriptor state type does not match the drawing arm's downcast",
                id.stable_name()
            );
        }
    }

    /// Every C-era scene names the `.c` its drawing half came from, and the one
    /// scene that has no oracle says so in words rather than naming a file that
    /// does not exist. A reviewer reading a placeholder card has to be able to
    /// tell "look here" from "there is nothing to look at".
    #[test]
    fn every_scene_names_its_source_and_the_one_without_an_oracle_says_so() {
        for id in SceneId::ALL {
            let source = oracle_source(id);
            if id.exists_in_oracle() {
                assert!(source.ends_with(".c"), "{}: {source}", id.stable_name());
            } else {
                assert!(
                    source.contains("no oracle"),
                    "{}: {source}",
                    id.stable_name()
                );
            }
        }
    }
}
