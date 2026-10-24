//! Each scene's identity colour: the swatch the plan lane, and anything else
//! that names a scene by colour, draws it in.
//!
//! **Content, not chrome**, so it does not change with the appearance (WX-5a).
//! Moved out of `panels/scene_timeline.rs` in WX-5b's seam commit so the
//! colour table and the lane that draws runs can have separate owners.
//!
//! **No scene wears the accent** (WX-5b). The accent means *now* — the
//! playhead, the run and section under it, the primary action — and the plan
//! lane draws every run in its scene's colour right under those marks, so a
//! scene colour that reads as the accent makes "now" ambiguous on exactly the
//! runs it lands on. Every colour here reads apart from both appearances'
//! accents by the rule the palette's own sweep uses (a 1.4:1 step of lightness
//! or 20° of hue, near-greys hue-less), and the test below walks every
//! [`SceneId`]. Song Atlas, Voronoi Bloom and Pixel Chapel moved for it; their
//! old ambers are kept in the test as the control.

use musializer_core::scene::SceneId;
use raylib::prelude::Color;

/// The scene's identity colour at `alpha`.
pub(crate) fn scene_color(scene: SceneId, alpha: u8) -> Color {
    let (r, g, b) = match scene {
        SceneId::Spectrum => (28, 117, 188),
        SceneId::PulseField => (213, 83, 73),
        SceneId::OrbitalLattice => (119, 82, 173),
        SceneId::AsciiField => (50, 125, 82),
        // Tideline's terrain olive. It was ochre (197,137,46), 2° of hue from
        // the dark accent, and it passed the accent rule only by being darker,
        // which the lane takes away by re-lighting every run toward its ground:
        // in a run it was a brown amber under an amber playhead. 26° off now.
        SceneId::SongAtlas => (140, 146, 70),
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
        // by sitting well into orange, and 21° red of the dark accent — the
        // swatch should *be* Clawd at a glance.
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
        // The window's ruby (its look is "a warm heart of gold and ruby in a
        // cobalt field"). Its gold (220,177,94) was the dark accent within 1° of
        // hue and 1.07:1 of lightness: the one colour on the lane a glance could
        // not tell from *now*.
        SceneId::VoronoiBloom => (196, 64, 98),
        SceneId::IridescentPool => (120, 185, 235),
        // The pixel stagings, stepped so neighbouring sections of the family
        // still read as two.
        SceneId::PixelBoot => (172, 200, 96),
        SceneId::PixelShell => (120, 220, 140),
        SceneId::PixelRunner => (200, 140, 230),
        SceneId::PixelQuartet => (230, 120, 110),
        // Candlelight on parchment, a yellow 23° clear of the dark accent. The
        // old warm gold (220,190,120) sat 4° from it at 1.04:1.
        SceneId::PixelChapel => (200, 202, 130),
        SceneId::PixelRave => (240, 110, 200),
        // The lattice's own aqua.
        SceneId::GlyphLattice => (96, 222, 200),
    };
    Color::new(r, g, b, alpha)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ui::theme::palette;
    use musializer_core::ui::contrast;

    fn packed(color: Color) -> u32 {
        u32::from_be_bytes([color.r, color.g, color.b, color.a])
    }

    /// `theme::tests::reads_apart`, restated because that module's helpers are
    /// private to its tests: a 1.4:1 step of lightness, or 20° of hue, with a
    /// near-grey (chroma under a fifth of its maximum) counting as hue-less.
    pub(crate) fn reads_apart(a: u32, b: u32) -> bool {
        contrast::ratio(a, b) >= 1.4 || hue_distance(a, b) >= 20.0
    }

    pub(crate) fn hue_distance(a: u32, b: u32) -> f64 {
        let hue = |value: u32| {
            let [r, g, b, _] = value.to_be_bytes();
            let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max - min;
            if max == 0.0 || delta / max < 0.2 {
                return None;
            }
            let h = if max == r {
                ((g - b) / delta).rem_euclid(6.0)
            } else if max == g {
                (b - r) / delta + 2.0
            } else {
                (r - g) / delta + 4.0
            };
            Some(h * 60.0)
        };
        match (hue(a), hue(b)) {
            (Some(x), Some(y)) => {
                let d = (x - y).abs();
                d.min(360.0 - d)
            }
            _ => 180.0,
        }
    }

    #[test]
    fn no_scene_colour_reads_as_either_accent() {
        let mut measured = 0;
        for scene in SceneId::ALL {
            let colour = packed(scene_color(scene, 255));
            for (appearance, accent) in [
                ("dark", palette::DARK.accent),
                ("light", palette::LIGHT.accent),
            ] {
                assert!(
                    reads_apart(colour, accent),
                    "{} reads as the {appearance} accent: {:.2}:1, {:.1}° of hue",
                    scene.display_name(),
                    contrast::ratio(colour, accent),
                    hue_distance(colour, accent)
                );
                measured += 1;
            }
        }
        assert_eq!(measured, SceneId::ALL.len() * 2);
    }

    /// The negative control, kept as the colours it came from: the two ambers
    /// the plan lane drew before WX-5b fail the rule against the dark accent.
    ///
    /// The third colour that moved, Song Atlas's ochre (197,137,46), is *not*
    /// one: it clears the rule by lightness alone, at 1.61:1 and 2° of hue. It
    /// moved because the lane re-lights every run toward its ground, so a
    /// darker amber stops being darker there and is only amber. Pinned below as
    /// the measurement it is, so a later reading of "it passed" has the numbers.
    #[test]
    fn the_ambers_the_lane_drew_before_would_read_as_the_accent() {
        let accent = palette::DARK.accent;
        for (name, old) in [
            ("Voronoi Bloom", 0xDCB1_5EFFu32),
            ("Pixel Chapel", 0xDCBE_78FF),
        ] {
            assert!(
                !reads_apart(old, accent),
                "{name}'s old gold now reads apart"
            );
        }
        let atlas = 0xC589_2EFFu32;
        assert!(hue_distance(atlas, accent) < 5.0);
        assert!((contrast::ratio(atlas, accent) - 1.61).abs() < 0.01);
    }
}
