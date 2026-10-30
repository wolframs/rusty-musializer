//! Pixel filter choices and column geometry. The picker deliberately uses the
//! same values and fallback order as the retired toolkit inspector.
use super::workspace_layout::UiRect;
use crate::project::pixel::{Matching, Pixel, PixelPalette};

/// What a pixel-look picker can say (2026-10-24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelChoice {
    /// A section wearing the video's look.
    Video,
    Off,
    Palette(PixelPalette),
    Levels,
    /// Pixels in the scene's own colours.
    Bare,
    /// A colour list a treatment wrote; shown, never offered.
    Listed,
}

impl PixelChoice {
    pub fn of(look: &Pixel) -> Self {
        if !look.is_on() {
            return Self::Off;
        }
        if let Some(palette) = look.named_palette() {
            return Self::Palette(palette);
        }
        match look.matching {
            Matching::None => Self::Bare,
            Matching::Levels => Self::Levels,
            Matching::Ramp | Matching::Nearest => Self::Listed,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "The video's look",
            Self::Off => "Off",
            Self::Palette(PixelPalette::Handheld) => "Handheld green",
            Self::Palette(PixelPalette::Grey) => "Handheld grey",
            Self::Palette(PixelPalette::Home) => "Home computer",
            Self::Palette(PixelPalette::Fantasy) => "Fantasy console",
            Self::Levels => "Console colour",
            Self::Bare => "Pixels only",
            Self::Listed => "Listed colours",
        }
    }
}

impl PixelChoice {
    pub const OFFERED: [Self; 7] = [
        Self::Off,
        Self::Palette(PixelPalette::Handheld),
        Self::Palette(PixelPalette::Grey),
        Self::Palette(PixelPalette::Home),
        Self::Palette(PixelPalette::Fantasy),
        Self::Levels,
        Self::Bare,
    ];

    /// Preserve every untouched field, including a custom screen and kick.
    pub fn choose(self, current: Option<Pixel>, video: Pixel) -> Option<Pixel> {
        let effective = current.unwrap_or(video);
        let start = [effective, video]
            .into_iter()
            .find(Pixel::is_on)
            .unwrap_or_else(Pixel::on);
        match self {
            Self::Video => None,
            Self::Off => Some(Pixel::OFF),
            Self::Palette(palette) => Some(start.with_palette(palette)),
            Self::Levels => Some(start.without_colours(Matching::Levels)),
            Self::Bare => Some(start.without_colours(Matching::None)),
            Self::Listed => Some(effective),
        }
    }
}

pub fn kick(look: Pixel, enabled: bool) -> Pixel {
    let cartridge = Pixel::cartridge();
    let (split, shake, zoom) = if enabled {
        (cartridge.split, cartridge.shake, cartridge.zoom)
    } else {
        (0.0, 0.0, 0.0)
    };
    Pixel {
        split,
        shake,
        zoom,
        ..look
    }
}

pub const HEIGHT: f32 = 290.0;
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub head: UiRect,
    pub invitation: UiRect,
    pub scope: UiRect,
    pub choices: [UiRect; 4],
    pub size: UiRect,
    pub kick: UiRect,
    pub status: UiRect,
}
pub fn layout(area: UiRect) -> Layout {
    let row = |y, h| UiRect::new(area.x, area.y + y, area.width, h);
    Layout {
        head: row(0.0, 18.0),
        invitation: row(24.0, 20.0),
        scope: row(50.0, 26.0),
        choices: [
            row(84.0, 26.0),
            row(116.0, 26.0),
            row(148.0, 26.0),
            row(180.0, 26.0),
        ],
        size: row(212.0, 30.0),
        kick: row(248.0, 26.0),
        status: row(278.0, 12.0),
    }
}
pub fn appearance_rect(footer: UiRect) -> UiRect {
    UiRect::new(
        footer.x + footer.width - 120.0,
        footer.y,
        120.0,
        footer.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn column_edits_serialize_byte_identically_to_the_toolkit_values() {
        use crate::project::{io, model::tests_support::valid_project};
        // Values the toolkit emits, explicitly pinned independently of choose().
        let old = [
            Pixel::OFF,
            Pixel::on(),
            Pixel::on().with_palette(PixelPalette::Grey),
            Pixel::on().with_palette(PixelPalette::Home),
            Pixel::on().with_palette(PixelPalette::Fantasy),
            Pixel::on().without_colours(Matching::Levels),
            Pixel::on().without_colours(Matching::None),
        ];
        for (choice, old_value) in PixelChoice::OFFERED.into_iter().zip(old) {
            let mut column = valid_project();
            column
                .scene_switches
                .cues
                .push(crate::project::model::SceneSwitchSuggestion {
                    scene_name: "spectrum".to_owned(),
                    id: 1,
                    start_seconds: 0.0,
                    end_seconds: 60.0,
                    ..Default::default()
                });
            let mut toolkit = column.clone();
            column.scene_switches.finish.pixel =
                choice.choose(Some(Pixel::OFF), Pixel::OFF).unwrap();
            toolkit.scene_switches.finish.pixel = old_value;
            assert_eq!(
                io::serialize(&column).unwrap(),
                io::serialize(&toolkit).unwrap(),
                "{choice:?}"
            );
            // Existing section overrides: None is inherit, OFF is an explicit override.
            for value in [
                None,
                Some(Pixel::OFF),
                Some(Pixel {
                    pixels: 144,
                    ..old_value
                }),
            ] {
                column.scene_switches.cues[0].pixel = value;
                toolkit.scene_switches.cues[0].pixel = value;
                assert_eq!(
                    io::serialize(&column).unwrap(),
                    io::serialize(&toolkit).unwrap()
                );
            }
        }
    }

    #[test]
    fn exact_layout_and_nearest_enabled_look() {
        let l = layout(UiRect::new(14.0, 300.0, 292.0, HEIGHT));
        assert_eq!(l.head, UiRect::new(14.0, 300.0, 292.0, 18.0));
        assert_eq!(l.scope, UiRect::new(14.0, 350.0, 292.0, 26.0));
        assert_eq!(
            l.choices,
            [
                UiRect::new(14.0, 384.0, 292.0, 26.0),
                UiRect::new(14.0, 416.0, 292.0, 26.0),
                UiRect::new(14.0, 448.0, 292.0, 26.0),
                UiRect::new(14.0, 480.0, 292.0, 26.0)
            ]
        );
        assert_eq!(l.size, UiRect::new(14.0, 512.0, 292.0, 30.0));
        assert_eq!(l.kick, UiRect::new(14.0, 548.0, 292.0, 26.0));
        assert_eq!(l.status, UiRect::new(14.0, 578.0, 292.0, 12.0));
        assert_eq!(
            appearance_rect(UiRect::new(150.0, 600.0, 156.0, 26.0)),
            UiRect::new(186.0, 600.0, 120.0, 26.0)
        );
        let video = Pixel {
            pixels: 144,
            ..Pixel::on()
        };
        assert_eq!(
            PixelChoice::Levels.choose(None, video),
            Some(video.without_colours(Matching::Levels))
        );
        assert_eq!(PixelChoice::Video.choose(Some(video), Pixel::OFF), None);
        assert_eq!(PixelChoice::Off.choose(None, video), Some(Pixel::OFF));
        assert_eq!(
            PixelChoice::Palette(PixelPalette::Handheld).choose(Some(Pixel::OFF), Pixel::OFF),
            Some(Pixel::on())
        );
        let owned = Pixel {
            pixels: 96,
            ..video
        };
        assert_eq!(
            PixelChoice::Bare.choose(Some(owned), video),
            Some(owned.without_colours(Matching::None))
        );
        assert_eq!(
            kick(owned, false),
            Pixel {
                split: 0.0,
                shake: 0.0,
                zoom: 0.0,
                ..owned
            }
        );
        assert_eq!(
            kick(kick(owned, false), true),
            Pixel {
                split: Pixel::cartridge().split,
                shake: Pixel::cartridge().shake,
                zoom: Pixel::cartridge().zoom,
                ..owned
            }
        );
    }
}
