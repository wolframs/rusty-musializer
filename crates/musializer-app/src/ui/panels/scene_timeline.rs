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
    SplitAt { seconds: f64 },
    RetimeBoundary { right_cue_id: u64, seconds: f64 },
    Retarget { cue_id: u64, scene: SceneId },
    CaptureTuning { cue_id: u64 },
    Remove { cue_id: u64 },
    SetEnabled(bool),
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
    };
    Color::new(r, g, b, alpha)
}

fn scene_text_color(scene: SceneId, enabled: bool) -> Color {
    if !enabled {
        return color::ui_ink();
    }
    let fill = packed(scene_color(scene, 255));
    if contrast::ratio(rgba::WHITE, fill) >= contrast::ratio(rgba::UI_INK, fill) {
        color::white()
    } else {
        color::ui_ink()
    }
}

fn packed(color: Color) -> u32 {
    u32::from_be_bytes([color.r, color.g, color.b, color.a])
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

        let name = selected_scene.map_or("Select a segment", SceneId::display_name);
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

            let disabled = contrast::blend(fill, rgba::UI_RAISED, 92.0 / 255.0);
            assert!(
                contrast::ratio(rgba::UI_INK, disabled) >= contrast::AA_TEXT,
                "{} disabled label",
                scene.display_name()
            );
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
