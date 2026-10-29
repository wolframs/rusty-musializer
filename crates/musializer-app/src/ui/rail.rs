//! The rail and its flyouts (WX-5c, `docs/WX5C_DESIGN.md` §1).
//!
//! The rail replaces the 320 px sidebar: four icons beside the preview and the
//! transport — Tracks and Scenes at the top, Editor and Dock at the bottom —
//! and the two that held the sidebar's panels open a **flyout** over the
//! preview's left edge instead of taking width from it for good. A flyout is
//! transient: it closes on Escape, on a press outside it, or on its icon, and
//! while it is open no press inside it reaches what it covers.
//!
//! The geometry is `musializer_core::ui::workspace_layout::WorkspaceRail`,
//! pure and pinned by `tools/differential_layout.sh`. This file is the
//! drawing and the open/close rules.

use musializer_core::scene::SceneId;
use musializer_core::ui::scene_browser::{PickerLayout, SceneFamily};
use musializer_core::ui::workspace_layout::{RailIcon, UiRect};
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::icons;
use super::scene_colors::scene_color;
use super::shell::{Flyout, Shell, ShellCommand, ShellInput};
use super::shell_layout::WorkspaceFrame;
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle};

/// How far the flyout's shadow falls on the preview to its right. The notice
/// tray starts past it, so a card is never drawn in the shadow.
pub(crate) const FLYOUT_SHADOW: f32 = 14.0;

/// One button in the scenes flyout's ASCII image footer (D2), and the gap
/// between the tiles and it.
const ASCII_BUTTON_HEIGHT: f32 = 22.0;
const ASCII_FOOTER_GAP: f32 = 8.0;
/// Clear's seat. It is reserved with or without an image, so importing never
/// makes Import grow under a press aimed at Clear.
const ASCII_CLEAR_WIDTH: f32 = 64.0;

/// A tile's name sizes, largest first: the label size, then down to the
/// caption size and one below it before a word is allowed to be clipped.
const TILE_FONT_SIZES: [f32; 3] = [
    metric::UI_FONT_LABEL,
    metric::UI_FONT_CAPTION,
    metric::UI_FONT_MICRO + 1.0,
];
/// Left inset of a tile's name: the colour strip, and air either side of it.
const TILE_STRIP_X: f32 = 7.0;
const TILE_STRIP_WIDTH: f32 = 3.0;
const TILE_TEXT_X: f32 = 17.0;

impl Shell {
    /// Decides, before anything is drawn, whether the open flyout survives
    /// this frame, and returns its rect if so.
    ///
    /// Run first because a press outside the flyout must close it on the same
    /// frame — otherwise the frame would draw the press's target occluded and
    /// the press would land on nothing. A press on a rail icon is not
    /// "outside": the icon itself toggles on release, and closing here would
    /// let that release reopen what the press just closed.
    pub(crate) fn flyout_begin(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
    ) -> Option<UiRect> {
        let Some(rail) = frame.rail_layout else {
            // Fullscreen and the editor hide the rail, and its flyouts with
            // it; coming back shows the plain workspace.
            self.open_flyout = None;
            return None;
        };
        self.open_flyout?;
        let rect = rail.flyout;
        let pointer = self.widgets.pointer(d);
        let on_icon = rail
            .icons
            .iter()
            .flatten()
            .any(|icon| icon.contains_point(pointer.x, pointer.y));
        if pointer.pressed && !on_icon && !rect.contains_point(pointer.x, pointer.y) {
            self.open_flyout = None;
            return None;
        }
        Some(rect)
    }

    /// The rail: its surface, a rule on its right edge, and four icons.
    ///
    /// Ghost buttons ([`ButtonStyle::Ghost`]): a glyph in muted ink until it
    /// is hovered, and the selection look — outline and tint, never the accent
    /// — while the state it names holds: its flyout open, the editor open, the
    /// dock open.
    pub(crate) fn rail(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(rail) = frame.rail_layout else {
            return;
        };
        let rect = rail.rail;
        if rect.is_empty() {
            return;
        }
        widgets::fill(d, rect, color::ui_surface());
        d.draw_line_ex(
            Vector2::new(rect.x + rect.width - 0.5, rect.y),
            Vector2::new(rect.x + rect.width - 0.5, rect.y + rect.height),
            1.0,
            color::ui_rule(),
        );

        for (slot, icon) in RailIcon::ALL.into_iter().enumerate() {
            let Some(boundary) = rail.icons[slot] else {
                continue;
            };
            let (control, selected) = match icon {
                RailIcon::Tracks => (icons::RAIL_TRACKS, self.open_flyout == Some(Flyout::Tracks)),
                RailIcon::Scenes => (icons::RAIL_SCENES, self.open_flyout == Some(Flyout::Scenes)),
                RailIcon::Editor => (icons::EDITOR, self.editor_open),
                RailIcon::Dock => (icons::DOCK, self.dock_open),
            };
            let id = widgets::widget_id(widgets::id::RAIL, slot as u32);
            let font_size = (boundary.height * 0.44).round();
            let state = match icons::glyph(input.fonts, &control) {
                icons::Glyph::Icon(face, text) => self.widgets.icon_button(
                    d,
                    face,
                    id,
                    boundary,
                    &text,
                    selected,
                    ButtonStyle::Ghost,
                    font_size,
                ),
                icons::Glyph::Text(font, text) => self.widgets.text_button(
                    d,
                    font,
                    id,
                    boundary,
                    &text,
                    selected,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_MICRO),
                ),
            };
            self.widgets.hint(d, state, id, boundary, control.tip);
            if !state.clicked {
                continue;
            }
            match icon {
                RailIcon::Tracks => self.toggle_flyout(Flyout::Tracks),
                RailIcon::Scenes => self.toggle_flyout(Flyout::Scenes),
                RailIcon::Editor => {
                    if self.lyric_draft_allows_context_change(input.workspace) {
                        self.open_flyout = None;
                        self.editor_open = true;
                    }
                }
                RailIcon::Dock => commands.push(self.toggle_dock()),
            }
        }
    }

    /// Opens `flyout`, or closes it if it is the one already open. Opening
    /// the other one replaces it: there is one flyout seat.
    pub(crate) fn toggle_flyout(&mut self, flyout: Flyout) {
        self.open_flyout = if self.open_flyout == Some(flyout) {
            None
        } else {
            Some(flyout)
        };
    }

    /// The open flyout, over everything but the AI settings modal and the
    /// tooltip. Drawn with this frame's plain input: it is the one surface the
    /// pointer is not occluded from.
    pub(crate) fn flyout(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        frame: &WorkspaceFrame,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        let (Some(kind), Some(rail)) = (self.open_flyout, frame.rail_layout) else {
            return;
        };
        let rect = rail.flyout;
        if rect.is_empty() {
            return;
        }
        // A soft edge on the preview side, so the flyout reads as lifted off
        // the work rather than as a panel that pushed it aside.
        d.draw_rectangle_gradient_h(
            (rect.x + rect.width) as i32,
            rect.y as i32,
            FLYOUT_SHADOW as i32,
            rect.height as i32,
            Color::new(0, 0, 0, 70),
            Color::new(0, 0, 0, 0),
        );
        match kind {
            Flyout::Tracks => self.tracks_panel(d, rect, input, commands),
            Flyout::Scenes => self.scene_browser(d, rect, input, commands),
        }
        d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, color::ui_control_edge());
        self.last_flyout = Some(rect);
    }

    /// The scenes flyout's body: the family column, the browsed family's
    /// tiles, and the ASCII image footer (WX-5c §1).
    ///
    /// A family row changes navigation only; it never retargets the selected
    /// plan section or the playing scene. A tile is a scene: its identity
    /// colour as a strip down its left edge, its name in up to two lines, and
    /// the selection look while it is the scene on screen. The flyout stays
    /// open after a pick, so trying scene after scene against the music is
    /// one press each.
    pub(crate) fn scene_browser(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        boundary: UiRect,
        input: &ShellInput<'_>,
        commands: &mut Vec<ShellCommand>,
    ) {
        if boundary.is_empty() {
            return;
        }
        let font = input.fonts.ui();
        let heading = format!("SCENES \u{00b7} {}", input.scene.display_name());
        let content = widgets::panel(d, font, boundary, &heading);
        let padding = metric::UI_PANEL_PADDING;
        let footer = UiRect::new(
            content.x + padding,
            content.y + content.height - padding - ASCII_BUTTON_HEIGHT,
            (content.width - padding * 2.0).max(0.0),
            ASCII_BUTTON_HEIGHT,
        );
        let area = UiRect::new(
            content.x + padding,
            content.y + padding,
            (content.width - padding * 2.0).max(0.0),
            (footer.y - ASCII_FOOTER_GAP - content.y - padding).max(0.0),
        );
        if area.is_empty() {
            self.scene_picker_report = "not drawn (no room)".to_owned();
            return;
        }
        self.scene_picker.observe_scene(input.scene);
        if let Some(family) = self.probe_family {
            self.scene_picker.browse(family);
        }
        let current_family = SceneFamily::of(input.scene);
        let mouse_allowed = !self.editor_open && !self.assist_settings.is_open();

        let mut layout = PickerLayout::new(area, self.scene_picker.family.scenes().len());
        let mut tabs = Vec::with_capacity(SceneFamily::ALL.len());
        for (index, family) in SceneFamily::ALL.into_iter().enumerate() {
            let rect = layout.tab(index);
            let widget = widgets::widget_id(widgets::id::SCENE_FAMILY, index as u32);
            let state = self.widgets.button(d, widget, rect);
            let browsed = family == self.scene_picker.family;
            let look = widgets::button_look(ButtonStyle::Ghost, browsed, state.hovered);
            widgets::control_box(d, rect, look.fill, look.edge, 1.0);
            let label_size = metric::UI_FONT_CAPTION;
            let y = rect.y + (rect.height - label_size) * 0.5;
            widgets::draw_text(
                d,
                font,
                family.label(),
                rect.x + 8.0,
                y,
                label_size,
                look.ink,
            );
            // How many scenes the family holds, and a dot on the family the
            // playing scene belongs to, so the column says where "now" is
            // without browsing to it. Ink rather than the accent: a family
            // is not the playhead.
            let count = family.scenes().len().to_string();
            let count_width = widgets::measure(font, &count, metric::UI_FONT_MICRO);
            let label_width = widgets::measure(font, family.label(), label_size);
            let count_x = rect.x + rect.width - 8.0 - count_width;
            if rect.x + 8.0 + label_width + 6.0 <= count_x {
                widgets::draw_text(
                    d,
                    font,
                    &count,
                    count_x,
                    rect.y + (rect.height - metric::UI_FONT_MICRO) * 0.5,
                    metric::UI_FONT_MICRO,
                    color::ui_muted(),
                );
            }
            if family == current_family {
                d.draw_circle_v(
                    Vector2::new(rect.x + 3.5, rect.y + rect.height * 0.5),
                    1.75,
                    color::ui_ink(),
                );
            }
            let names = family
                .scenes()
                .iter()
                .map(|scene| scene.display_name())
                .collect::<Vec<_>>()
                .join(", ");
            self.widgets.hint(
                d,
                state,
                widget,
                rect,
                &format!("{}: {}", family.label(), names),
            );
            if state.clicked {
                self.scene_picker.browse(family);
                self.probe_family = None;
            }
            tabs.push(format!("{}@{}", family.stable_name(), rect_token(rect)));
        }

        // A row press changes what is browsed, so the tiles are laid out for
        // the family the press left, in the same frame.
        let scenes = self.scene_picker.family.scenes();
        layout = PickerLayout::new(area, scenes.len());
        self.scene_picker.clamp(layout);
        let mouse = input.ui_scale.mouse(d);
        if layout.viewport.contains_point(mouse.x, mouse.y) && mouse_allowed {
            let wheel = self.wheel_delta(d);
            self.scene_picker.wheel(wheel, layout);
        }
        if let Some(bar) = layout.scrollbar() {
            let id = widgets::widget_id(widgets::id::SCENE_FAMILY, 20);
            let state = self.widgets.button(d, id, bar);
            if state.pressed && mouse_allowed {
                let thumb_height = layout
                    .thumb(self.scene_picker.offset)
                    .map_or(0.0, |thumb| thumb.height);
                let fraction =
                    (mouse.y - bar.y - thumb_height * 0.5) / (bar.height - thumb_height).max(1.0);
                self.scene_picker.scrollbar_to(fraction, layout);
            }
        }

        let plan_is_driving = input
            .workspace
            .current()
            .is_some_and(|track| track.scene_switches.enabled && !track.scene_switches.is_empty());
        let mut visible = Vec::new();
        let mut tiles = Vec::new();
        let mut smallest = TILE_FONT_SIZES[0];
        let old_clip = self.widgets.set_input_clip(Some(layout.viewport));
        {
            let mut clip = widgets::begin_scissor(d, layout.viewport, input.ui_scale);
            for (index, id) in scenes.iter().copied().enumerate() {
                let rect = layout.tile(index, self.scene_picker.offset);
                let hit = rect.intersect(layout.viewport);
                if hit.is_empty() {
                    continue;
                }
                visible.push(id.stable_name());
                tiles.push(format!("{}@{}", id.stable_name(), rect_token(hit)));
                // Stable scene widget ids survive grouping, scrolling and the
                // addition of a new family. Hidden pixels cannot claim a click.
                let widget = widgets::widget_id(widgets::id::SCENE_BROWSER, id.index() as u32);
                let state = self.widgets.button(&clip, widget, hit);
                let selected = id == input.scene;
                let look = widgets::button_look(ButtonStyle::Neutral, selected, state.hovered);
                let fill = if state.pressed {
                    widgets::brightness(look.fill, -0.08)
                } else {
                    look.fill
                };
                widgets::control_box(&mut clip, rect, fill, look.edge, 1.0);
                widgets::fill(
                    &mut clip,
                    UiRect::new(
                        rect.x + TILE_STRIP_X,
                        rect.y + 8.0,
                        TILE_STRIP_WIDTH,
                        (rect.height - 16.0).max(0.0),
                    ),
                    scene_color(id, 255),
                );
                let width = rect.width - TILE_TEXT_X - 6.0;
                let (lines, size) =
                    tile_lines(id, width, |text, size| widgets::measure(font, text, size));
                smallest = smallest.min(size);
                let leading = size + 2.0;
                let block = leading * lines.len() as f32 - 2.0;
                let mut y = rect.y + (rect.height - block) * 0.5;
                for line in &lines {
                    widgets::draw_text(
                        &mut clip,
                        font,
                        line,
                        rect.x + TILE_TEXT_X,
                        y,
                        size,
                        look.ink,
                    );
                    y += leading;
                }
                self.widgets.hint(
                    &clip,
                    state,
                    widget,
                    hit,
                    crate::scene_host::scene_picker_hint(id),
                );
                if state.clicked && (id != input.scene || plan_is_driving) {
                    commands.push(ShellCommand::SelectScene(id));
                }
            }
        }
        self.widgets.set_input_clip(old_clip);
        if let Some(bar) = layout.scrollbar() {
            widgets::fill(d, bar, color::ui_rule());
            if let Some(thumb) = layout.thumb(self.scene_picker.offset) {
                widgets::fill(d, thumb, color::ui_muted());
            }
        }
        let (import, clear) = self.ascii_image_footer(d, input, footer, commands);
        self.scene_picker_report = format!(
            "family={} active={} visible=[{}] offset={:.1}/{:.1} viewport={:.0}x{:.0}@{:.0},{:.0} \
             font={:.1} tabs={} tiles={} footer=import@{};clear@{}",
            self.scene_picker.family.stable_name(),
            input.scene.stable_name(),
            visible.join(","),
            self.scene_picker.offset,
            layout.max_scroll,
            layout.viewport.width,
            layout.viewport.height,
            layout.viewport.x,
            layout.viewport.y,
            smallest,
            tabs.join(";"),
            if tiles.is_empty() {
                "none".to_owned()
            } else {
                tiles.join(";")
            },
            rect_token(import),
            rect_token(clear),
        );
    }

    /// The scenes flyout's "Import image" / "Clear" pair (D2,
    /// `plug.c:6358-6394`), returning both seats for the report line.
    ///
    /// Clear is drawn **only when the current track owns an image-backed
    /// grid**, which is the oracle's condition: ASCII Field always has
    /// something to draw — its procedural spectrogram — so a Clear with
    /// nothing to clear would offer to undo a state the user is not in. Its
    /// seat is reported either way, so a probe can press the empty seat and
    /// prove nothing is there.
    fn ascii_image_footer(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        footer: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> (UiRect, UiRect) {
        let font = input.fonts.ui();
        let has_image = input
            .workspace
            .current()
            .is_some_and(|track| track.ascii.is_some());
        let gap = 4.0;
        // Keep the absent Clear seat inert. Import must not grow underneath
        // it and turn a previously harmless press into a file-dialog launch.
        let import = UiRect::new(
            footer.x,
            footer.y,
            (footer.width - ASCII_CLEAR_WIDTH - gap).max(0.0),
            footer.height,
        );
        let clear = UiRect::new(
            footer.x + footer.width - ASCII_CLEAR_WIDTH,
            footer.y,
            ASCII_CLEAR_WIDTH,
            footer.height,
        );
        if footer.is_empty() {
            return (import, clear);
        }
        let import_id = widgets::widget_id(widgets::id::SCENE_ASCII, 0);
        let state = self.widgets.text_button(
            d,
            font,
            import_id,
            import,
            "Import image",
            false,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(
            d,
            state,
            import_id,
            import,
            "Convert a PNG, JPEG or BMP into ASCII Field's glyph grid",
        );
        if state.clicked {
            commands.push(ShellCommand::ImportAsciiImageDialog);
        }
        if !has_image {
            return (import, clear);
        }
        let clear_id = widgets::widget_id(widgets::id::SCENE_ASCII, 1);
        let state = self.widgets.text_button(
            d,
            font,
            clear_id,
            clear,
            "Clear",
            false,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(
            d,
            state,
            clear_id,
            clear,
            "Drop the imported image and go back to the procedural grid",
        );
        if state.clicked {
            commands.push(ShellCommand::ClearAsciiImage);
        }
        (import, clear)
    }

    /// The `rail:` report line (WX-5c §1): the rail of the last workspace
    /// frame, its icons, the open flyout and where it drew.
    ///
    /// `icons=` entries are `name@x,y,w,h` separated by `;`, like the scene
    /// picker's `tiles=`, because the rects carry commas of their own. A probe
    /// that presses an icon reads its point from here.
    #[must_use]
    pub fn describe_rail(&self) -> String {
        match self.last_rail {
            None => "none".to_owned(),
            Some(rail) => {
                let icons: Vec<String> = RailIcon::ALL
                    .into_iter()
                    .zip(rail.icons)
                    .filter_map(|(icon, rect)| {
                        rect.map(|rect| format!("{}@{}", icon.name(), rect_token(rect)))
                    })
                    .collect();
                format!(
                    "rect={} icons={} open={} flyout={}",
                    rect_token(rail.rail),
                    if icons.is_empty() {
                        "none".to_owned()
                    } else {
                        icons.join(";")
                    },
                    self.open_flyout.map_or("none", Flyout::name),
                    self.last_flyout.map_or("none".to_owned(), rect_token),
                )
            }
        }
    }
}

/// A scene's name broken into at most two lines that fit `width`, and the
/// size it fits at: the largest of [`TILE_FONT_SIZES`] at which every line
/// fits, else the smallest, where a word longer than the tile is left to the
/// tile's clip. Greedy, on spaces: "Spectral Terrarium" is two lines,
/// "Clawd" one.
fn tile_lines(
    scene: SceneId,
    width: f32,
    measure: impl Fn(&str, f32) -> f32,
) -> (Vec<String>, f32) {
    let name = scene.display_name();
    let wrap = |size: f32| -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for word in name.split(' ') {
            match lines.last_mut() {
                Some(line) if measure(&format!("{line} {word}"), size) <= width => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push(word.to_owned()),
            }
        }
        lines
    };
    for size in TILE_FONT_SIZES {
        let lines = wrap(size);
        if lines.len() <= 2 && lines.iter().all(|line| measure(line, size) <= width) {
            return (lines, size);
        }
    }
    let size = TILE_FONT_SIZES[TILE_FONT_SIZES.len() - 1];
    let mut lines = wrap(size);
    if lines.len() > 2 {
        let rest = lines.split_off(1).join(" ");
        lines.push(rest);
    }
    (lines, size)
}

/// `x,y,w,h`, rounded to whole logical pixels, as every report line prints a
/// rect.
pub(crate) fn rect_token(rect: UiRect) -> String {
    format!(
        "{:.0},{:.0},{:.0},{:.0}",
        rect.x, rect.y, rect.width, rect.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::ui::workspace_layout::WorkspaceRail;

    #[test]
    fn a_rail_icon_toggles_its_flyout_and_the_other_one_replaces_it() {
        let mut shell = Shell::new();
        shell.toggle_flyout(Flyout::Tracks);
        assert_eq!(shell.open_flyout, Some(Flyout::Tracks));
        shell.toggle_flyout(Flyout::Scenes);
        assert_eq!(shell.open_flyout, Some(Flyout::Scenes));
        shell.toggle_flyout(Flyout::Scenes);
        assert_eq!(shell.open_flyout, None);
    }

    /// Character counts stand in for a face: 7 px a character at 14, so the
    /// wrap is checked as arithmetic rather than as a picture.
    #[test]
    fn a_tile_name_wraps_on_spaces_into_at_most_two_lines() {
        let measure = |text: &str, size: f32| text.chars().count() as f32 * size * 0.5;
        assert_eq!(
            tile_lines(SceneId::SpectralTerrarium, 80.0, measure),
            (vec!["Spectral".to_owned(), "Terrarium".to_owned()], 14.0)
        );
        assert_eq!(
            tile_lines(SceneId::Clawd, 80.0, measure),
            (vec!["Clawd".to_owned()], 14.0)
        );
        // "Terrarium" is 63 px at 14 and 58.5 at 13: a 60 px tile takes 13.
        assert_eq!(
            tile_lines(SceneId::SpectralTerrarium, 60.0, measure).1,
            13.0
        );
        // Three words that need three lines at every size keep two.
        let (lines, size) = tile_lines(SceneId::Pentagram, 30.0, measure);
        assert_eq!(lines.len(), 2);
        assert_eq!(size, 12.0);
    }

    #[test]
    fn the_rail_line_names_every_icon_and_the_flyout_it_drew() {
        let mut shell = Shell::new();
        assert_eq!(shell.describe_rail(), "none");
        shell.last_rail = WorkspaceRail::layout(1280.0, 720.0, 46.0, 460.0);
        shell.open_flyout = Some(Flyout::Scenes);
        shell.last_flyout = Some(UiRect::new(52.0, 46.0, 336.0, 674.0));
        assert_eq!(
            shell.describe_rail(),
            "rect=0,46,52,414 icons=tracks@8,56,36,36;scenes@8,98,36,36;\
             editor@8,372,36,36;dock@8,414,36,36 open=scenes flyout=52,46,336,674"
        );
    }
}
