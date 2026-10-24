//! Each scene's identity colour: the swatch the plan lane, and anything else
//! that names a scene by colour, draws it in.
//!
//! **Content, not chrome**, so it does not change with the appearance (WX-5a).
//! Moved out of `panels/scene_timeline.rs` in WX-5b's seam commit so the
//! colour table and the lane that draws runs can have separate owners: the
//! table is being retuned off the accent's hue (Song Atlas, Voronoi Bloom and
//! Pixel Chapel sit near the dark accent's amber) while the lane is rebuilt.

use musializer_core::scene::SceneId;
use raylib::prelude::Color;

/// The scene's identity colour at `alpha`.
pub(crate) fn scene_color(scene: SceneId, alpha: u8) -> Color {
    let (r, g, b) = match scene {
        SceneId::Spectrum => (28, 117, 188),
        SceneId::PulseField => (213, 83, 73),
        SceneId::OrbitalLattice => (119, 82, 173),
        SceneId::AsciiField => (50, 125, 82),
        SceneId::SongAtlas => (197, 137, 46),
        SceneId::SpectralTerrarium => (40, 151, 137),
        SceneId::Constellation => (67, 91, 173),
        SceneId::Cadence => (185, 73, 137),
        SceneId::Loom => (139, 102, 55),
        SceneId::Pentagram => (106, 106, 116),
        // A phosphor cyan, deliberately clear of ASCII Field's green and
        // Spectral Terrarium's teal: the scene lane identifies a segment by this
        // swatch alone, so two scenes a glance cannot separate would make the
        // plan unreadable.
        SceneId::PhosphorDream => (0, 148, 176),
        // The character's own terracotta. Clear of Pulse Field's red (213,83,73)
        // by sitting well into orange, and of Song Atlas's ochre (197,137,46) by
        // staying red of it — the swatch should *be* Clawd at a glance.
        SceneId::Clawd => (222, 112, 68),
        SceneId::OpenAiKnot => (131, 89, 230),
        SceneId::Stage => (214, 92, 148),
        // The door's lavender.
        SceneId::Threshold => (156, 144, 200),
        SceneId::FluidInk => (100, 210, 160),
        SceneId::LiquidChrome => (180, 194, 218),
        SceneId::AuroraVeil => (100, 210, 185),
        SceneId::MetaballGarden => (225, 140, 185),
        SceneId::NeonTunnel => (160, 130, 240),
        SceneId::VoronoiBloom => (220, 177, 94),
        SceneId::IridescentPool => (120, 185, 235),
        // The pixel stagings share the handheld's light green, stepped so
        // neighbouring sections of the family still read as two.
        SceneId::PixelBoot => (172, 200, 96),
        SceneId::PixelShell => (120, 220, 140),
        SceneId::PixelRunner => (200, 140, 230),
        SceneId::PixelQuartet => (230, 120, 110),
        SceneId::PixelChapel => (220, 190, 120),
        SceneId::PixelRave => (240, 110, 200),
    };
    Color::new(r, g, b, alpha)
}
