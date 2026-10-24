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

pub const TAB_HEIGHT: f32 = 22.0;
pub const GAP: f32 = 4.0;
pub const TILE_FLOOR: f32 = 24.0;
/// The family tabs sit in two rows whatever their number, so a new family
/// costs tab width rather than tile height: the first half of
/// [`SceneFamily::ALL`] (rounded up) on the top row, the rest below.
const TAB_ROWS: usize = 2;

/// Where a family tab sits: its row, its column in that row, how many tabs
/// share the row, and the fraction of the row's free width before it and its
/// own. Widths are proportional to the label's length plus a fixed allowance
/// for the button's padding, so the longest label in a row is not the one
/// squeezed. Character counts stand in for measured widths because this module
/// has no font; the shell fits one font size over the widths this produces.
fn tab_slot(index: usize) -> Option<(usize, usize, usize, f32, f32)> {
    let count = SceneFamily::ALL.len();
    if index >= count {
        return None;
    }
    let per_row = count.div_ceil(TAB_ROWS);
    let row = index / per_row;
    let first = row * per_row;
    let last = (first + per_row).min(count);
    let weight = |i: usize| SceneFamily::ALL[i].label().chars().count() as f32 + 3.0;
    let total: f32 = (first..last).map(weight).sum();
    let before: f32 = (first..index).map(weight).sum();
    Some((
        row,
        index - first,
        last - first,
        before / total,
        weight(index) / total,
    ))
}

#[derive(Clone, Copy, Debug)]
pub struct PickerLayout {
    pub area: UiRect,
    pub viewport: UiRect,
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
        let top = TAB_ROWS as f32 * TAB_HEIGHT + GAP * TAB_ROWS as f32;
        let viewport = UiRect::new(
            area.x,
            area.y + top,
            area.width,
            (area.height - top).max(0.0),
        );
        let columns = if area.width >= 170.0 { 2 } else { 1 };
        let rows = count.div_ceil(columns).max(1);
        let tile_height =
            ((viewport.height - GAP * (rows - 1) as f32) / rows as f32).clamp(TILE_FLOOR, 44.0);
        let content_height = rows as f32 * tile_height + (rows - 1) as f32 * GAP;
        let max_scroll = (content_height - viewport.height).max(0.0);
        let gutter = if max_scroll > 0.0 { 12.0 } else { 0.0 };
        let tile_width =
            ((area.width - gutter - GAP * (columns - 1) as f32) / columns as f32).max(0.0);
        Self {
            area,
            viewport,
            columns,
            tile_height,
            tile_width,
            max_scroll,
            content_height,
            count,
        }
    }
    #[must_use]
    pub fn tab(self, index: usize) -> UiRect {
        let Some((row, column, in_row, start, share)) = tab_slot(index) else {
            return UiRect::new(self.area.x, self.area.y, 0.0, 0.0);
        };
        let gaps = GAP * in_row.saturating_sub(1) as f32;
        let free = (self.area.width - gaps).max(0.0);
        UiRect::new(
            self.area.x + start * free + column as f32 * GAP,
            self.area.y + row as f32 * (TAB_HEIGHT + GAP),
            share * free,
            TAB_HEIGHT,
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
    #[test]
    fn every_tile_is_reachable_without_shrinking_at_small_and_future_sizes() {
        for width in [150.0, 228.0, 308.0, 388.0] {
            for height in [104.0, 148.0, 220.0, 300.0] {
                for count in [2, 5, 22, 100] {
                    let area = UiRect::new(6.0, 30.0, width, height);
                    let layout = PickerLayout::new(area, count);
                    assert!(layout.tile_height >= 24.0);
                    for i in 0..SceneFamily::ALL.len() {
                        assert!(area.contains(layout.tab(i)));
                        assert!(layout.tab(i).width > 0.0);
                        for j in 0..i {
                            assert!(
                                layout.tab(i).intersect(layout.tab(j)).is_empty(),
                                "tabs {i} and {j} overlap"
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
    #[test]
    fn grouping_removes_the_growth_pressure_on_the_default_picker() {
        let area = UiRect::new(6.0, 30.0, 228.0, 148.0);
        assert!(PickerLayout::new(area, SceneId::ALL.len()).max_scroll > 0.0);
        for family in SceneFamily::ALL {
            assert_eq!(
                PickerLayout::new(area, family.scenes().len()).max_scroll,
                0.0
            );
        }
    }
}
