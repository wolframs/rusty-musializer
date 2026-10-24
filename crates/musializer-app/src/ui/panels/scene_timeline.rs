//! The always-visible scene-plan lane and its compact editing controls.
//!
//! Pixels and immediate-mode state live here; hit testing and boundary clamping
//! live in `musializer-core::ui::scene_lane_edit`. Durable mutations leave as
//! [`ScenePlanEdit`] commands and are applied by `main.rs`.

use musializer_core::scene::SceneId;
use musializer_core::ui::contrast;
use musializer_core::ui::scene_lane_edit::{self, SceneLaneZone};
use musializer_core::ui::timed_lane;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::super::shell::{Shell, ShellCommand, ShellInput, TimelineGesture};
use super::super::theme::{color, metric, rgba};
use super::super::widgets::{self, ButtonStyle};

pub const SCENE_CONTROLS_HEIGHT: f32 = 26.0;
pub const SCENE_LANE_HEIGHT: f32 = 24.0;

/// Where this section's lane starts, measured from the top of the area it is
/// handed.
///
/// Exported because `shell.rs` needs the *top of the first timed lane* to draw
/// the group frame, and deriving it there from
/// [`SCENE_SECTION_HEIGHT`] minus the pieces would be the same arithmetic
/// written twice.
pub const SCENE_LANE_OFFSET: f32 = SCENE_CONTROLS_HEIGHT + metric::LANE_GAP;

/// Controls row, gap, lane, **and a trailing gap** (LX1-e).
///
/// The trailing [`metric::LANE_GAP`] is what puts a seam between this lane and
/// the waveform strip below it. Before LX1-e the section ended flush against
/// the lane's bottom border, so the strip's top border landed on the very next
/// row and the two lanes drew as one box with a 2 px rule through it — the
/// "glued together" the operator called out.
///
/// It has to be spent *here* rather than in `timeline_strip`: `timeline_height`
/// adds this constant to whatever the open panel asked for, so a gap inside the
/// section grows the band with it, while a gap added below the section would be
/// taken out of the lyrics editor — which has a compile-time assertion against
/// exactly that.
pub const SCENE_SECTION_HEIGHT: f32 = SCENE_LANE_OFFSET + SCENE_LANE_HEIGHT + metric::LANE_GAP;

/// A durable scene-plan mutation. Stable cue ids cross the draw/model boundary;
/// vector indexes do not, because split and delete renumber them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScenePlanEdit {
    SplitAt {
        seconds: f64,
    },
    RetimeBoundary {
        right_cue_id: u64,
        seconds: f64,
    },
    Retarget {
        cue_id: u64,
        scene: SceneId,
    },
    CaptureTuning {
        cue_id: u64,
    },
    Remove {
        cue_id: u64,
    },
    SetEnabled(bool),
    /// One section's pixel look (2026-10-24): `None` wears the video's,
    /// `Some(Pixel::OFF)` switches it off there. Written by the Editor's Tune
    /// tab; the same field a treatment's section `pixel` compiles into.
    SetPixel {
        cue_id: u64,
        pixel: Option<musializer_core::project::pixel::Pixel>,
    },
    /// The whole video's pixel look: the finish's `pixel`, which applies with
    /// or without a plan.
    SetVideoPixel(musializer_core::project::pixel::Pixel),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BoundaryDrag {
    right_cue_id: u64,
    origin_x: f32,
    preview_seconds: f64,
    moved: bool,
}

/// Session-only selection and drag preview. No project state is duplicated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneLaneEditor {
    pub selected_id: u64,
    drag: Option<BoundaryDrag>,
    /// A body press remains a selection click until it crosses the common drag
    /// threshold, then becomes the timeline's transactional scrub.
    scrub_origin_x: Option<f32>,
    /// A successful split is applied after drawing; resolve its new segment on
    /// the following frame without guessing the model's next id in the UI.
    pending_select_seconds: Option<f64>,
}

fn scene_color(scene: SceneId, alpha: u8) -> Color {
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

fn scene_text_color(scene: SceneId, enabled: bool) -> Color {
    if !enabled {
        return color::ui_ink();
    }
    let fill = packed(scene_color(scene, 255));
    if contrast::ratio(rgba::WHITE, fill) >= contrast::ratio(rgba::INK_ON_CONTENT, fill) {
        color::white()
    } else {
        Color::get_color(rgba::INK_ON_CONTENT)
    }
}

fn packed(color: Color) -> u32 {
    u32::from_be_bytes([color.r, color.g, color.b, color.a])
}

/// Clips a label to `width` with an ellipsis, so a long segment summary stays
/// inside its field.
fn fit_text(input: &ShellInput<'_>, text: &str, width: f32) -> String {
    if widgets::measure(input.fonts.ui(), text, metric::UI_FONT_CAPTION) <= width {
        return text.to_owned();
    }
    let mut kept: Vec<char> = text.chars().collect();
    while !kept.is_empty() {
        kept.pop();
        let candidate: String = kept.iter().collect::<String>() + "…";
        if widgets::measure(input.fonts.ui(), &candidate, metric::UI_FONT_CAPTION) <= width {
            return candidate;
        }
    }
    String::new()
}

/// What a segment carries besides its scene, drawn on its block (MV9-k):
/// the transition it arrives by as a light band fading in from its left edge
/// across the transition's length, a glide as a ramp rising across the glide,
/// and a grade as a strip of its palette along the bottom. Marks only; nothing
/// here is editable yet.
fn draw_segment_marks(
    d: &mut RaylibDrawHandle<'_>,
    timeline: &musializer_core::ui::timeline_view::TimelineView,
    lane: UiRect,
    rect: UiRect,
    cue: &musializer_core::project::scene_switch::SceneSwitchCue,
    start: f64,
    alpha: u8,
) {
    let span_px = |seconds: f32| {
        let to = timeline.x_at(
            start + f64::from(seconds),
            f64::from(lane.x),
            f64::from(lane.width),
        ) as f32;
        (to - rect.x).clamp(0.0, rect.width)
    };
    if !cue.transition.is_cut() {
        let width = span_px(cue.transition.seconds).max(3.0);
        d.draw_rectangle_gradient_h(
            rect.x as i32,
            rect.y as i32,
            width as i32,
            3,
            Color::new(255, 236, 196, alpha),
            Color::new(255, 236, 196, 0),
        );
    }
    if cue.glide > 0.0 {
        let width = span_px(cue.glide);
        if width >= 2.0 {
            let bottom = rect.y + rect.height - 1.0;
            d.draw_line_ex(
                Vector2::new(rect.x, bottom),
                Vector2::new(rect.x + width, rect.y + 4.0),
                1.5,
                Color::new(255, 255, 255, alpha / 2),
            );
        }
    }
    if let Some(grade) = cue.grade.filter(|grade| !grade.is_identity()) {
        let stops = grade.stops();
        let y = (rect.y + rect.height - 3.0) as i32;
        let pieces = stops.len().saturating_sub(1).max(1);
        let piece = rect.width / pieces as f32;
        for index in 0..pieces {
            let [r0, g0, b0] = musializer_core::project::look::rgb(stops[index]);
            let [r1, g1, b1] =
                musializer_core::project::look::rgb(stops[(index + 1).min(stops.len() - 1)]);
            let channel = |value: f32| (value * 255.0).round() as u8;
            d.draw_rectangle_gradient_h(
                (rect.x + piece * index as f32) as i32,
                y,
                piece.ceil() as i32,
                3,
                Color::new(channel(r0), channel(g0), channel(b0), alpha),
                Color::new(channel(r1), channel(g1), channel(b1), alpha),
            );
        }
    }
    // The section's own pixel look (2026-10-24): a little checker of its
    // palette in the block's top-right corner, so a lane reads "this is where
    // the four-shade world unlocks" at a glance; a hollow square for a section
    // that switches the look off. A section wearing the video's look has no
    // mark — the finish is the whole lane's, and says so in the name field.
    if let Some(look) = cue.pixel {
        let size = 4.0f32;
        let x = rect.x + rect.width - size * 3.0 - 3.0;
        let y = rect.y + 3.0;
        if rect.width < size * 5.0 {
            return;
        }
        if look.is_on() {
            let colours = look.colours();
            let shade = |index: usize| -> Color {
                let packed = match colours.len() {
                    0 => [0x000000, 0x555555, 0xaaaaaa, 0xffffff][index],
                    count => colours[(index * (count - 1)) / 3],
                };
                let [r, g, b] = musializer_core::project::look::rgb(packed);
                let channel = |value: f32| (value * 255.0).round() as u8;
                Color::new(channel(r), channel(g), channel(b), alpha)
            };
            for (column, row, index) in [
                (0, 0, 0),
                (1, 0, 3),
                (2, 0, 1),
                (0, 1, 3),
                (1, 1, 2),
                (2, 1, 3),
            ] {
                d.draw_rectangle(
                    (x + size * column as f32) as i32,
                    (y + size * row as f32) as i32,
                    size as i32,
                    size as i32,
                    shade(index),
                );
            }
        } else {
            d.draw_rectangle_lines(
                x as i32,
                y as i32,
                (size * 3.0) as i32,
                (size * 2.0) as i32,
                Color::new(255, 255, 255, alpha),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn control_button(
    shell: &mut Shell,
    d: &mut RaylibDrawHandle<'_>,
    input: &ShellInput<'_>,
    row: UiRect,
    x: &mut f32,
    gap: f32,
    index: u32,
    width: f32,
    label: &str,
    enabled: bool,
    selected: bool,
    style: ButtonStyle,
) -> widgets::ButtonState {
    let rect = UiRect::new(*x, row.y, width, row.height);
    *x += width + gap;
    if !enabled {
        widgets::fill(d, rect, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, color::ui_rule());
        let text_width = widgets::measure(input.fonts.ui(), label, metric::UI_FONT_CAPTION);
        widgets::draw_text(
            d,
            input.fonts.ui(),
            label,
            rect.x + ((rect.width - text_width) * 0.5).max(3.0),
            rect.y + 6.0,
            metric::UI_FONT_CAPTION,
            color::ui_disabled(),
        );
        return widgets::ButtonState::default();
    }
    shell.widgets.text_button(
        d,
        input.fonts.ui(),
        widgets::widget_id(widgets::id::TIMELINE, 20 + index),
        rect,
        label,
        selected,
        style,
        Some(metric::UI_FONT_CAPTION),
    )
}

impl Shell {
    /// Draws controls and the aligned scene blocks, returning the fixed height
    /// consumed. The fixed budget prevents selection from making the preview
    /// jump vertically.
    pub(crate) fn scene_plan_section(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> f32 {
        let Some(track) = input.workspace.current() else {
            return 0.0;
        };
        if let Some(seconds) = self.scene_lane.pending_select_seconds.take() {
            self.scene_lane.selected_id = track
                .scene_switches
                .cues()
                .iter()
                .find(|cue| cue.start_seconds <= seconds && seconds < cue.end_seconds)
                .map_or(0, |cue| cue.id);
        }
        if area.width <= 0.0 || area.height < SCENE_SECTION_HEIGHT {
            return 0.0;
        }
        if self.scene_lane.selected_id != 0
            && !track
                .scene_switches
                .cues()
                .iter()
                .any(|cue| cue.id == self.scene_lane.selected_id)
        {
            self.scene_lane.selected_id = 0;
        }

        let controls = UiRect::new(area.x, area.y, area.width, SCENE_CONTROLS_HEIGHT);
        let lane = UiRect::new(
            area.x,
            area.y + SCENE_LANE_OFFSET,
            area.width,
            SCENE_LANE_HEIGHT,
        );
        self.scene_plan_controls(d, input, controls, commands);
        self.scene_plan_lane(d, input, lane, commands);
        SCENE_SECTION_HEIGHT
    }

    fn scene_plan_controls(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        let gap = 6.0;
        let widths = [68.0, 78.0, 28.0, 28.0, 104.0, 64.0];
        let fixed: f32 = widths.iter().sum::<f32>() + gap * 6.0;
        let name_width = (row.width - fixed).max(72.0);
        if fixed + name_width > row.width {
            return;
        }
        let selected = track
            .scene_switches
            .cues()
            .iter()
            .find(|cue| cue.id == self.scene_lane.selected_id);
        let selected_scene = selected.and_then(|cue| SceneId::from_index(cue.scene_index as usize));
        let mut x = row.x;

        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            0,
            widths[0],
            if track.scene_switches.enabled {
                "Auto on"
            } else {
                "Auto off"
            },
            !track.scene_switches.is_empty(),
            track.scene_switches.enabled,
            ButtonStyle::Neutral,
        )
        .clicked
            && !track.scene_switches.is_empty()
        {
            commands.push(ShellCommand::ScenePlan(ScenePlanEdit::SetEnabled(
                !track.scene_switches.enabled,
            )));
        }
        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            1,
            widths[1],
            "Split here",
            true,
            false,
            ButtonStyle::Neutral,
        )
        .clicked
        {
            self.scene_lane.pending_select_seconds = Some(input.time_seconds);
            commands.push(ShellCommand::ScenePlan(ScenePlanEdit::SplitAt {
                seconds: input.time_seconds,
            }));
        }
        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            2,
            widths[2],
            "<",
            selected.is_some(),
            false,
            ButtonStyle::Neutral,
        )
        .clicked
        {
            if let (Some(cue), Some(scene)) = (selected, selected_scene) {
                let previous =
                    SceneId::ALL[(scene.index() + SceneId::ALL.len() - 1) % SceneId::ALL.len()];
                commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Retarget {
                    cue_id: cue.id,
                    scene: previous,
                }));
            }
        }

        // The name field carries what a segment's colour cannot (MV9-k): a
        // threshold's beyond, how it arrives, a glide, a grade. With nothing
        // selected it says whether the video has a finish, which is otherwise
        // invisible everywhere in the window.
        let detail =
            selected.and_then(|cue| scene_lane_edit::segment_summary(cue, &track.scene_settings));
        let name_owned = match (selected_scene, detail) {
            (Some(scene), Some(detail)) => format!("{} · {detail}", scene.display_name()),
            (Some(scene), None) => scene.display_name().to_owned(),
            (None, _) if track.scene_switches.finish.pixel.is_on() => {
                let look = track.scene_switches.finish.pixel;
                format!(
                    "Select a segment · pixel look {} {}",
                    look.palette_label(),
                    look.pixels
                )
            }
            (None, _) if track.scene_switches.finish.is_active() => {
                "Select a segment · film finish on".to_owned()
            }
            (None, _) => "Select a segment".to_owned(),
        };
        let name = fit_text(input, &name_owned, name_width - 8.0);
        let name = name.as_str();
        let name_rect = UiRect::new(x, row.y, name_width, row.height);
        widgets::fill(d, name_rect, color::ui_raised());
        d.draw_rectangle_lines_ex(widgets::rectangle(name_rect), 1.0, color::ui_rule());
        let name_text_width = widgets::measure(input.fonts.ui(), name, metric::UI_FONT_CAPTION);
        widgets::draw_text(
            d,
            input.fonts.ui(),
            name,
            name_rect.x + ((name_rect.width - name_text_width) * 0.5).max(4.0),
            name_rect.y + 6.0,
            metric::UI_FONT_CAPTION,
            if selected.is_some() {
                color::ui_ink()
            } else {
                color::ui_disabled()
            },
        );
        x += name_width + gap;

        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            3,
            widths[3],
            ">",
            selected.is_some(),
            false,
            ButtonStyle::Neutral,
        )
        .clicked
        {
            if let (Some(cue), Some(scene)) = (selected, selected_scene) {
                let next = SceneId::ALL[(scene.index() + 1) % SceneId::ALL.len()];
                commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Retarget {
                    cue_id: cue.id,
                    scene: next,
                }));
            }
        }
        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            4,
            widths[4],
            "Capture tuning",
            selected.is_some(),
            false,
            ButtonStyle::Neutral,
        )
        .clicked
        {
            if let Some(cue) = selected {
                commands.push(ShellCommand::ScenePlan(ScenePlanEdit::CaptureTuning {
                    cue_id: cue.id,
                }));
            }
        }
        if control_button(
            self,
            d,
            input,
            row,
            &mut x,
            gap,
            5,
            widths[5],
            "Delete",
            selected.is_some(),
            false,
            ButtonStyle::Danger,
        )
        .clicked
        {
            if let Some(cue) = selected {
                commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Remove {
                    cue_id: cue.id,
                }));
            }
        }
    }

    fn scene_plan_lane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        lane: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        self.timeline_pan_gesture(d, input, lane);
        self.scene_lane_gesture(d, input, lane, commands);
        widgets::fill(d, lane, color::ui_raised());
        d.draw_rectangle_lines_ex(
            widgets::rectangle(lane),
            metric::LANE_BORDER,
            color::ui_rule(),
        );

        let drag = self.scene_lane.drag;
        // The section under the playhead wears the accent as an inset outline
        // (WX-5a, the mockup's `.run.now`): it is the "current section" the
        // accent exists to mark, read off the same playhead the lane draws.
        let playhead = self.timeline_playhead_seconds(input.time_seconds);
        for (index, cue) in track.scene_switches.cues().iter().enumerate() {
            let mut start = cue.start_seconds;
            let mut end = cue.end_seconds;
            if let Some(drag) = drag {
                if cue.id == drag.right_cue_id {
                    start = drag.preview_seconds;
                }
                if track
                    .scene_switches
                    .cues()
                    .get(index + 1)
                    .is_some_and(|next| next.id == drag.right_cue_id)
                {
                    end = drag.preview_seconds;
                }
            }
            let Some(block) = timed_lane::block_geometry(
                &self.timeline,
                f64::from(lane.x),
                f64::from(lane.width),
                start,
                end,
            ) else {
                continue;
            };
            let Some(scene) = SceneId::from_index(cue.scene_index as usize) else {
                continue;
            };
            let selected = cue.id == self.scene_lane.selected_id;
            let alpha = if track.scene_switches.enabled {
                255
            } else {
                92
            };
            let text_color = scene_text_color(scene, track.scene_switches.enabled);
            let rect = UiRect::new(
                block.left as f32,
                lane.y + 2.0,
                block.width() as f32,
                lane.height - 4.0,
            );
            widgets::fill(d, rect, scene_color(scene, alpha));
            d.draw_rectangle_lines_ex(
                widgets::rectangle(rect),
                if selected { 2.0 } else { 1.0 },
                text_color,
            );
            let now = track.scene_switches.enabled
                && cue.start_seconds <= playhead
                && playhead < cue.end_seconds;
            if now && rect.width > 4.0 {
                let inset = if selected { 2.0 } else { 1.0 };
                d.draw_rectangle_lines_ex(
                    widgets::rectangle(UiRect::new(
                        rect.x + inset,
                        rect.y + inset,
                        rect.width - inset * 2.0,
                        rect.height - inset * 2.0,
                    )),
                    1.0,
                    color::accent(),
                );
            }
            let label = scene.display_name();
            let label_width = widgets::measure(input.fonts.ui(), label, metric::UI_FONT_CAPTION);
            if label_width + 10.0 <= rect.width {
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    label,
                    rect.x + 5.0,
                    rect.y + 3.0,
                    metric::UI_FONT_CAPTION,
                    text_color,
                );
            }
            draw_segment_marks(d, &self.timeline, lane, rect, cue, start, alpha);
        }
        // How the track ends (MV3): a band fading to black across the outro.
        let outro = track.scene_switches.outro;
        if !outro.is_cut() && track.duration_seconds > 0.0 {
            let begin = (track.duration_seconds - f64::from(outro.seconds)).max(0.0);
            let from =
                self.timeline
                    .x_at(begin, f64::from(lane.x), f64::from(lane.width)) as f32;
            let to = self.timeline.x_at(
                track.duration_seconds,
                f64::from(lane.x),
                f64::from(lane.width),
            ) as f32;
            let (from, to) = (from.max(lane.x), to.min(lane.x + lane.width));
            if to > from {
                d.draw_rectangle_gradient_h(
                    from as i32,
                    (lane.y + 2.0) as i32,
                    (to - from).max(1.0) as i32,
                    (lane.height - 4.0) as i32,
                    Color::new(0, 0, 0, 0),
                    Color::new(0, 0, 0, 200),
                );
            }
        }

        if track.scene_switches.is_empty() {
            widgets::draw_text(
                d,
                input.fonts.ui(),
                "No scene plan yet - Split adds the current scene at the playhead",
                lane.x + 6.0,
                lane.y + 5.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }

        if let Some(drag) = drag {
            let x = self.timeline.x_at(
                drag.preview_seconds,
                f64::from(lane.x),
                f64::from(lane.width),
            ) as f32;
            d.draw_line_ex(
                Vector2::new(x, lane.y - 2.0),
                Vector2::new(x, lane.y + lane.height + 2.0),
                2.0,
                color::ui_ink(),
            );
        }

        // No playhead here since LX1-e. `Shell::timeline_group_chrome` draws one
        // marker across every timed lane at [`metric::LANE_PLAYHEAD_WIDTH`],
        // after the last of them: this lane used to draw its own at 1 px, the
        // strip below drew a second at 2 px and the cue lane a third at 1 px, so
        // one moment in time was marked three times at two weights with a break
        // in each gap.
    }

    fn scene_lane_gesture(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        lane: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        use raylib::consts::MouseButton::MOUSE_BUTTON_LEFT;

        let Some(track) = input.workspace.current() else {
            return;
        };
        let mouse = input.ui_scale.mouse(d);
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.scene_lane.scrub_origin_x = None;
            self.update_timeline_scrub(
                mouse.x,
                lane,
                track.duration_seconds,
                d.is_mouse_button_down(MOUSE_BUTTON_LEFT),
                commands,
            );
            return;
        }
        if let Some(origin_x) = self.scene_lane.scrub_origin_x {
            if !d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
                self.scene_lane.scrub_origin_x = None;
                return;
            }
            if (mouse.x - origin_x).abs() >= timed_lane::DRAG_THRESHOLD_PIXELS as f32 {
                self.scene_lane.scrub_origin_x = None;
                if self.begin_timeline_scrub(input.playing, commands) {
                    self.update_timeline_scrub(
                        mouse.x,
                        lane,
                        track.duration_seconds,
                        true,
                        commands,
                    );
                }
            }
            return;
        }
        if self.scene_lane.drag.is_none() {
            if self.timeline_gesture.is_some()
                || !lane.contains_point(mouse.x, mouse.y)
                || !d.is_mouse_button_pressed(MOUSE_BUTTON_LEFT)
            {
                return;
            }
            let hit = scene_lane_edit::hit_test(
                &track.scene_switches,
                &self.timeline,
                f64::from(lane.x),
                f64::from(lane.width),
                f64::from(mouse.x),
            );
            let Some(hit) = hit else {
                self.scene_lane.selected_id = 0;
                self.scene_lane.scrub_origin_x = Some(mouse.x);
                return;
            };
            self.scene_lane.selected_id = hit.id;
            if hit.zone == SceneLaneZone::Boundary {
                let start = track
                    .scene_switches
                    .cues()
                    .iter()
                    .find(|cue| cue.id == hit.id)
                    .map_or(0.0, |cue| cue.start_seconds);
                self.scene_lane.drag = Some(BoundaryDrag {
                    right_cue_id: hit.id,
                    origin_x: mouse.x,
                    preview_seconds: start,
                    moved: false,
                });
                self.timeline_gesture = Some(TimelineGesture::SceneBoundary);
            } else {
                self.scene_lane.scrub_origin_x = Some(mouse.x);
            }
            return;
        }

        if self.timeline_gesture != Some(TimelineGesture::SceneBoundary) {
            self.scene_lane.drag = None;
            return;
        }
        let mut drag = self.scene_lane.drag.expect("checked above");
        if d.is_mouse_button_down(MOUSE_BUTTON_LEFT) {
            if (mouse.x - drag.origin_x).abs() >= timed_lane::DRAG_THRESHOLD_PIXELS as f32 {
                drag.moved = true;
            }
            if drag.moved {
                let proposed = self.timeline.seconds_at(
                    f64::from(mouse.x),
                    f64::from(lane.x),
                    f64::from(lane.width),
                    track.duration_seconds,
                );
                if let Some(clamped) = scene_lane_edit::clamp_boundary(
                    &track.scene_switches,
                    drag.right_cue_id,
                    proposed,
                ) {
                    drag.preview_seconds = clamped;
                }
            }
            self.scene_lane.drag = Some(drag);
            return;
        }

        if drag.moved {
            commands.push(ShellCommand::ScenePlan(ScenePlanEdit::RetimeBoundary {
                right_cue_id: drag.right_cue_id,
                seconds: drag.preview_seconds,
            }));
        }
        self.scene_lane.drag = None;
        self.timeline_gesture = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::palette;

    #[test]
    fn every_scene_label_clears_body_text_contrast_enabled_and_disabled() {
        for scene in SceneId::ALL {
            let fill = packed(scene_color(scene, 255));
            let text = packed(scene_text_color(scene, true));
            assert!(
                contrast::ratio(text, fill) >= contrast::AA_TEXT,
                "{} enabled label",
                scene.display_name()
            );

            // A disabled plan draws its blocks translucent over the lane, so
            // the label is measured against that blend — in both appearances,
            // because the lane is a different colour in each (WX-5a).
            for palette in [&palette::DARK, &palette::LIGHT] {
                let disabled = contrast::blend(fill, palette.ui_raised, 92.0 / 255.0);
                assert!(
                    contrast::ratio(palette.ui_ink, disabled) >= contrast::AA_TEXT,
                    "{} disabled label on {:06X}",
                    scene.display_name(),
                    palette.ui_raised >> 8
                );
            }
        }
    }

    #[test]
    fn a_fresh_editor_has_no_project_shaped_state() {
        let editor = SceneLaneEditor::default();
        assert_eq!(editor.selected_id, 0);
        assert!(editor.drag.is_none());
        assert!(editor.scrub_origin_x.is_none());
        assert!(editor.pending_select_seconds.is_none());
    }
}
