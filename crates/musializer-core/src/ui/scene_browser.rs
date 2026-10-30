//! Compact scene families and the picker's geometry, independent of registry ids.
//!
//! Groups are navigation, not scene order: saved ids, keyboard cycling and the
//! scene-plan lane keep the append-only registry. Only one family occupies the
//! tile viewport. Excess rows scroll rather than becoming smaller or disappearing.

use super::workspace_layout::UiRect;
use crate::scene::SceneId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SceneFamily {
    #[default]
    Sound,
    Geometry,
    Atmosphere,
    Materials,
    Living,
    TypeStage,
    /// The pixel-art stagings (2026-10-24): one family, six scenes.
    PixelCats,
}
impl SceneFamily {
    pub const ALL: [Self; 7] = [
        Self::Sound,
        Self::Geometry,
        Self::Atmosphere,
        Self::Materials,
        Self::Living,
        Self::TypeStage,
        Self::PixelCats,
    ];
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sound => "Sound",
            Self::Geometry => "Geometry",
            Self::Atmosphere => "Atmosphere",
            Self::Materials => "Materials",
            Self::Living => "Living",
            Self::TypeStage => "Type & stage",
            Self::PixelCats => "Pixel cats",
        }
    }
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Sound => "sound",
            Self::Geometry => "geometry",
            Self::Atmosphere => "atmosphere",
            Self::Materials => "materials",
            Self::Living => "living",
            Self::TypeStage => "type-stage",
            Self::PixelCats => "pixel-cats",
        }
    }
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.stable_name() == name)
    }
    #[must_use]
    pub fn of(scene: SceneId) -> Self {
        match scene {
            SceneId::Spectrum | SceneId::PulseField | SceneId::SongAtlas | SceneId::Loom => {
                Self::Sound
            }
            SceneId::OrbitalLattice
            | SceneId::Pentagram
            | SceneId::OpenAiKnot
            | SceneId::MetaballGarden
            | SceneId::VoronoiBloom
            | SceneId::GlyphLattice => Self::Geometry,
            SceneId::Constellation
            | SceneId::FluidInk
            | SceneId::AuroraVeil
            | SceneId::NeonTunnel => Self::Atmosphere,
            SceneId::LiquidChrome | SceneId::IridescentPool => Self::Materials,
            SceneId::SpectralTerrarium | SceneId::Clawd => Self::Living,
            SceneId::AsciiField
            | SceneId::PhosphorDream
            | SceneId::Cadence
            | SceneId::Stage
            | SceneId::Threshold => Self::TypeStage,
            SceneId::PixelBoot
            | SceneId::PixelShell
            | SceneId::PixelRunner
            | SceneId::PixelQuartet
            | SceneId::PixelChapel
            | SceneId::PixelRave => Self::PixelCats,
        }
    }
    #[must_use]
    pub fn scenes(self) -> Vec<SceneId> {
        SceneId::ALL
            .into_iter()
            .filter(|scene| Self::of(*scene) == self)
            .collect()
    }
}

/// The family column's width (WX-5c §1). Wide enough for "Type & stage" and
/// "Atmosphere" at the label size; the tiles take the rest of the flyout.
pub const FAMILY_WIDTH: f32 = 96.0;
/// A family row's height when the column has room for every row at it, and
/// the least it may shrink to.
pub const FAMILY_HEIGHT: f32 = 30.0;
pub const FAMILY_FLOOR: f32 = 18.0;
/// Between the family column and the tiles.
pub const COLUMN_GAP: f32 = 10.0;
/// Between two rows, or two tiles.
pub const GAP: f32 = 4.0;
/// A tile seats a real poster above its name, with a legible fallback state.
pub const TILE_FLOOR: f32 = 84.0;
pub const TILE_CEILING: f32 = 104.0;
/// The scrollbar's lane, taken from the tiles' width only when they scroll.
pub const SCROLL_GUTTER: f32 = 12.0;

/// The picker's geometry: a column of families on the left, one row each,
/// and the browsed family's tiles in a two-column grid on the right
/// (WX-5c §1).
///
/// **The shape changed on purpose.** In the 320 px sidebar the families were
/// two rows of label-width tabs over a viewport about 100 px tall, which is why
/// Pixel Cats stopped at six stagings — six was what a family showed
/// unscrolled at the minimum picker. In the flyout the tiles get the flyout's
/// height: at 960x640 a family shows ten poster tiles before it scrolls.
#[derive(Clone, Copy, Debug)]
pub struct PickerLayout {
    pub area: UiRect,
    /// The family column.
    pub families: UiRect,
    /// The tiles' viewport: what scrolls, and what clips a press.
    pub viewport: UiRect,
    pub family_height: f32,
    pub columns: usize,
    pub tile_height: f32,
    pub tile_width: f32,
    pub max_scroll: f32,
    pub content_height: f32,
    pub count: usize,
}
impl PickerLayout {
    #[must_use]
    pub fn new(area: UiRect, count: usize) -> Self {
        let family_width = FAMILY_WIDTH.min((area.width * 0.4).max(0.0));
        let families = UiRect::new(area.x, area.y, family_width, area.height.max(0.0));
        let rows = SceneFamily::ALL.len() as f32;
        let family_height = ((area.height - GAP * (rows - 1.0)) / rows)
            .clamp(FAMILY_FLOOR, FAMILY_HEIGHT)
            .floor();
        let viewport = UiRect::new(
            area.x + family_width + COLUMN_GAP,
            area.y,
            (area.width - family_width - COLUMN_GAP).max(0.0),
            area.height.max(0.0),
        );
        let columns = if viewport.width >= 170.0 { 2 } else { 1 };
        let rows = count.div_ceil(columns).max(1);
        let tile_height = ((viewport.height - GAP * (rows - 1) as f32) / rows as f32)
            .clamp(TILE_FLOOR, TILE_CEILING)
            .floor();
        let content_height = rows as f32 * tile_height + (rows - 1) as f32 * GAP;
        let max_scroll = (content_height - viewport.height).max(0.0);
        let gutter = if max_scroll > 0.0 { SCROLL_GUTTER } else { 0.0 };
        let tile_width = ((viewport.width - gutter - GAP * (columns - 1) as f32) / columns as f32)
            .max(0.0)
            .floor();
        Self {
            area,
            families,
            viewport,
            family_height,
            columns,
            tile_height,
            tile_width,
            max_scroll,
            content_height,
            count,
        }
    }
    /// The family row at `index` in [`SceneFamily::ALL`]: one row each,
    /// top to bottom, the column's full width.
    #[must_use]
    pub fn tab(self, index: usize) -> UiRect {
        if index >= SceneFamily::ALL.len() {
            return UiRect::new(self.families.x, self.families.y, 0.0, 0.0);
        }
        UiRect::new(
            self.families.x,
            self.families.y + index as f32 * (self.family_height + GAP),
            self.families.width,
            self.family_height,
        )
    }
    #[must_use]
    pub fn tile(self, index: usize, offset: f32) -> UiRect {
        UiRect::new(
            self.viewport.x + (index % self.columns) as f32 * (self.tile_width + GAP),
            self.viewport.y + (index / self.columns) as f32 * (self.tile_height + GAP)
                - offset.clamp(0.0, self.max_scroll),
            self.tile_width,
            self.tile_height,
        )
    }
    #[must_use]
    pub fn scrollbar(self) -> Option<UiRect> {
        (self.max_scroll > 0.0 && self.viewport.height > 0.0).then(|| {
            UiRect::new(
                self.viewport.x + self.viewport.width - 9.0,
                self.viewport.y,
                9.0,
                self.viewport.height,
            )
        })
    }
    #[must_use]
    pub fn thumb(self, offset: f32) -> Option<UiRect> {
        let bar = self.scrollbar()?;
        let height = (bar.height * bar.height / self.content_height)
            .max(14.0)
            .min(bar.height);
        let y =
            bar.y + (bar.height - height) * offset.clamp(0.0, self.max_scroll) / self.max_scroll;
        Some(UiRect::new(bar.x, y, bar.width, height))
    }
}

#[derive(Clone, Debug, Default)]
pub struct PickerState {
    pub family: SceneFamily,
    pub offset: f32,
    last_scene: Option<SceneId>,
}
impl PickerState {
    /// Reveal a scene selected through CLI, keyboard, track open or the plan.
    /// Merely browsing another family does not change the current scene.
    pub fn observe_scene(&mut self, scene: SceneId) {
        if self.last_scene != Some(scene) {
            self.family = SceneFamily::of(scene);
            self.offset = 0.0;
            self.last_scene = Some(scene);
        }
    }
    pub fn browse(&mut self, family: SceneFamily) {
        if self.family != family {
            self.family = family;
            self.offset = 0.0;
        }
    }
    pub fn wheel(&mut self, notches: f32, layout: PickerLayout) {
        if notches.is_finite() {
            self.offset =
                (self.offset - notches * (layout.tile_height + GAP)).clamp(0.0, layout.max_scroll);
        }
    }
    pub fn scrollbar_to(&mut self, fraction: f32, layout: PickerLayout) {
        if fraction.is_finite() {
            self.offset = fraction.clamp(0.0, 1.0) * layout.max_scroll;
        }
    }
    pub fn clamp(&mut self, layout: PickerLayout) {
        self.offset = self.offset.clamp(0.0, layout.max_scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_scene_has_exactly_one_family_without_reordering_saved_ids() {
        let listed: Vec<_> = SceneFamily::ALL
            .into_iter()
            .flat_map(SceneFamily::scenes)
            .collect();
        assert_eq!(listed.len(), SceneId::ALL.len());
        for scene in SceneId::ALL {
            assert_eq!(listed.iter().filter(|&&entry| entry == scene).count(), 1);
        }
        assert_eq!(SceneId::FluidInk.index(), 15);
        assert_eq!(
            SceneFamily::of(SceneId::LiquidChrome),
            SceneFamily::Materials
        );
        assert_eq!(
            SceneFamily::of(SceneId::NeonTunnel),
            SceneFamily::Atmosphere
        );
    }
    #[test]
    fn browsing_is_independent_and_a_new_active_scene_is_revealed() {
        let mut state = PickerState::default();
        state.observe_scene(SceneId::Spectrum);
        state.browse(SceneFamily::Materials);
        state.observe_scene(SceneId::Spectrum);
        assert_eq!(
            state.family,
            SceneFamily::Materials,
            "browsing must survive subsequent frames"
        );
        state.observe_scene(SceneId::VoronoiBloom);
        assert_eq!(state.family, SceneFamily::Geometry);
        assert_eq!(state.offset, 0.0);
    }
    /// The flyout's picker area at the 960x640 minimum: the flyout is 336
    /// wide and 594 tall, the panel header takes 27, 10 px of padding go
    /// round the area, and the ASCII footer and its gap take 30 at the bottom.
    const MINIMUM_AREA: UiRect = UiRect::new(62.0, 83.0, 316.0, 517.0);

    /// Pinned numbers, not properties: the picker at the minimum window.
    #[test]
    fn the_picker_at_the_minimum_window_is_a_family_column_and_two_tile_columns() {
        let layout = PickerLayout::new(MINIMUM_AREA, 6);
        assert_eq!(layout.families, UiRect::new(62.0, 83.0, 96.0, 517.0));
        assert_eq!(layout.viewport, UiRect::new(168.0, 83.0, 210.0, 517.0));
        assert_eq!(layout.family_height, 30.0);
        assert_eq!(layout.tab(0), UiRect::new(62.0, 83.0, 96.0, 30.0));
        assert_eq!(layout.tab(6), UiRect::new(62.0, 287.0, 96.0, 30.0));
        assert_eq!(layout.columns, 2);
        assert_eq!(layout.tile_height, 104.0);
        assert_eq!(layout.tile_width, 103.0);
        assert_eq!(layout.tile(0, 0.0), UiRect::new(168.0, 83.0, 103.0, 104.0));
        assert_eq!(layout.tile(5, 0.0), UiRect::new(275.0, 299.0, 103.0, 104.0));
        assert_eq!(layout.max_scroll, 0.0);
    }

    /// Real posters need height: ten scenes fit; larger families scroll.
    #[test]
    fn a_family_of_ten_posters_fits_and_larger_families_scroll() {
        for count in [6, 10] {
            let layout = PickerLayout::new(MINIMUM_AREA, count);
            assert_eq!(layout.max_scroll, 0.0, "{count} tiles scroll");
            assert!(layout.scrollbar().is_none());
            let last = layout.tile(count - 1, 0.0);
            assert!(layout.viewport.contains(last), "{count}: {last:?}");
        }
        assert_eq!(PickerLayout::new(MINIMUM_AREA, 20).tile_height, 84.0);
        assert_eq!(PickerLayout::new(MINIMUM_AREA, 20).max_scroll, 359.0);
        // Twenty-two rows of floor height do not fit 517 px, and scroll.
        assert!(PickerLayout::new(MINIMUM_AREA, 44).max_scroll > 0.0);
        // Every family as the scenes are registered today.
        for family in SceneFamily::ALL {
            let layout = PickerLayout::new(MINIMUM_AREA, family.scenes().len());
            assert_eq!(layout.max_scroll, 0.0, "{family:?}");
        }
    }

    #[test]
    fn every_tile_is_reachable_and_no_row_or_tile_overlaps() {
        for width in [240.0, 316.0, 400.0] {
            for height in [160.0, 240.0, 517.0, 800.0] {
                for count in [2, 5, 22, 100] {
                    let area = UiRect::new(6.0, 30.0, width, height);
                    let layout = PickerLayout::new(area, count);
                    assert!(layout.tile_height >= TILE_FLOOR);
                    assert!(!layout.families.overlaps(layout.viewport));
                    for i in 0..SceneFamily::ALL.len() {
                        assert!(area.contains(layout.tab(i)), "{width}x{height}: row {i}");
                        for j in 0..i {
                            assert!(
                                layout.tab(i).intersect(layout.tab(j)).is_empty(),
                                "rows {i} and {j} overlap"
                            );
                        }
                    }
                    for i in 0..count {
                        let top = layout.tile(i, 0.0).y - layout.viewport.y;
                        let offset = top.min(layout.max_scroll);
                        assert!(
                            layout.viewport.contains(layout.tile(i, offset)),
                            "{width}x{height}: tile {i}/{count} unreachable"
                        );
                    }
                    let mut state = PickerState::default();
                    state.wheel(-1000.0, layout);
                    assert_eq!(state.offset, layout.max_scroll);
                    state.wheel(1000.0, layout);
                    assert_eq!(state.offset, 0.0);
                    if let Some(thumb) = layout.thumb(layout.max_scroll) {
                        assert!(
                            (thumb.y + thumb.height - (layout.viewport.y + layout.viewport.height))
                                .abs()
                                < 0.01
                        );
                    }
                }
            }
        }
    }
}
