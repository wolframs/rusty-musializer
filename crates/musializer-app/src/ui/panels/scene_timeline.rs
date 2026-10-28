//! The always-visible scene-plan lane and its compact editing controls.
//!
//! Pixels and immediate-mode state live here; hit testing and boundary clamping
//! live in `musializer-core::ui::scene_lane_edit`. Durable mutations leave as
//! [`ScenePlanEdit`] commands and are applied by `main.rs`.

use musializer_core::scene::SceneId;
use musializer_core::ui::contrast;
use musializer_core::ui::lane_head::{LaneHead, LaneHeadWidths};
use musializer_core::ui::plan_runs::{fold_runs, RunSpan};
use musializer_core::ui::scene_lane_edit::{self, SceneLaneZone};
use musializer_core::ui::timed_lane;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};

use super::super::scene_colors::scene_color;
use super::super::shell::{Shell, ShellCommand, ShellInput, TimelineGesture};
use super::super::theme::{self, color, metric, Palette};
use super::super::widgets::{self, ButtonStyle};

/// The lane head's row (WX-5c §3): the plan controls row it replaced, at the
/// same height, so the scene section keeps its 80 px and nothing below it in
/// the band moves.
pub const SCENE_CONTROLS_HEIGHT: f32 = musializer_core::ui::lane_head::LANE_HEAD_HEIGHT;
/// 44 since WX-5b (it was 24): a run carries a 16:9 poster frame beside its
/// scene's name, and at 24 px the poster is a coloured smear. The band grows by
/// the 20 px with it — every budget below reads [`SCENE_SECTION_HEIGHT`].
pub const SCENE_LANE_HEIGHT: f32 = 44.0;

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
    /// The last body press, `(time, x)`, so a second one inside the
    /// double-click window opens the run under it (WX-5b).
    last_press: Option<(f64, f32)>,
    /// The run last opened (double-click or `run-open=`), by its first cue id.
    opened: Option<u64>,
    /// What the lane drew last frame, for the `scene runs:` report line.
    last_frame: RunFrame,
    /// What the lane head drew last frame, for the `lane head:` line.
    head_report: String,
}

/// How long two presses may be apart and still be one double-click, in
/// seconds, and how far apart in logical pixels. The desktop default is 400 ms;
/// the distance keeps a fast press-and-drag from reading as one.
const DOUBLE_CLICK_SECONDS: f64 = 0.4;
const DOUBLE_CLICK_PIXELS: f32 = 4.0;

/// Where the runs' tooltip ids start in the timeline namespace: clear of the
/// controls row's `20 + index` and the strip's `1`, and the namespace's
/// collision test walks 0..4096.
const RUN_HINT_IDS: u32 = 1000;

/// The margin either side of a run that a double-click opens, as a share of
/// the run, so its first and last cut are not on the lane's edge.
const OPEN_RUN_MARGIN: f64 = 0.06;

/// What the lane drew in one frame (WX-5b). Counted while drawing, so the
/// report line says what reached the screen rather than what the plan holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RunFrame {
    runs: usize,
    sections: usize,
    cuts: usize,
    /// Internal-cut hairlines actually drawn (visible, and wide enough).
    hairlines: usize,
    arriving: usize,
    graded: usize,
    /// `(run, section, scene)` under the playhead, while the plan is on.
    now: Option<(usize, usize, SceneId)>,
    selected: Option<usize>,
    opened: Option<u64>,
    posters_ready: usize,
    posters_total: usize,
    unavailable: usize,
    /// The lane's logical rect, so a probe can aim at a cut from the report
    /// rather than from a coordinate measured off a screenshot once.
    lane: UiRect,
    /// The view the lane drew, `start..end` in seconds.
    view: (f64, f64),
}

/// Whether a press at `(now, x)` completes a double-click begun by `last`.
fn is_double_click(last: Option<(f64, f32)>, now: f64, x: f32) -> bool {
    last.is_some_and(|(time, from)| {
        let elapsed = now - time;
        (0.0..=DOUBLE_CLICK_SECONDS).contains(&elapsed) && (x - from).abs() <= DOUBLE_CLICK_PIXELS
    })
}

impl SceneLaneEditor {
    /// The `lane head:` report line's body (WX-5c §3): the ladder's step, the
    /// selection, the plan label, and the rect of every control it drew, as
    /// `name=x,y,w,h`. Probes that press the head read their points here.
    #[must_use]
    pub fn describe_head(&self) -> &str {
        if self.head_report.is_empty() {
            "none"
        } else {
            &self.head_report
        }
    }

    /// The `scene runs:` report line's body (WX-5b).
    #[must_use]
    pub fn describe(&self) -> String {
        let frame = self.last_frame;
        let now = frame.now.map_or_else(
            || "none".to_owned(),
            |(run, section, scene)| {
                format!(
                    "run {}/{} section {} ({})",
                    run + 1,
                    frame.runs,
                    section + 1,
                    scene.stable_name()
                )
            },
        );
        format!(
            "runs={} sections={} cuts={} hairlines={} arriving={} graded={} now={now} \
             selected={} opened={} posters={}/{} unavailable={} \
             lane={:.0},{:.0},{:.0},{:.0} view={:.3}..{:.3}",
            frame.runs,
            frame.sections,
            frame.cuts,
            frame.hairlines,
            frame.arriving,
            frame.graded,
            frame
                .selected
                .map_or_else(|| "none".to_owned(), |index| (index + 1).to_string()),
            frame
                .opened
                .map_or_else(|| "none".to_owned(), |id| format!("cue {id}")),
            frame.posters_ready,
            frame.posters_total,
            frame.unavailable,
            frame.lane.x,
            frame.lane.y,
            frame.lane.width,
            frame.lane.height,
            frame.view.0,
            frame.view.1,
        )
    }
}

/// A run's colours: its fill top and bottom, the poster fade, and its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RunLook {
    top: Color,
    bottom: Color,
    mid: Color,
    edge: Color,
}

/// The scene colour as a run wears it (WX-5b, the mockup's `.run`).
///
/// On the dark ground the colour is mixed toward black, 50 % at the top to
/// 28 % at the bottom, so twenty-odd runs read as a calm strip with a poster in
/// each rather than as the 5a lane's saturated barcode; on the light ground it
/// is mixed toward white the same way round. The name is ink on both, which the
/// tests measure for every scene. A disabled plan sinks further toward the
/// ground; its names stay ink.
fn run_look(rgb: (u8, u8, u8), palette: &Palette, enabled: bool) -> RunLook {
    let dark = contrast::relative_luminance(palette.ui_surface) < 0.2;
    let mix = |share: f32| {
        let ground = if dark { 0.0 } else { 255.0 };
        let channel = |value: u8| (f32::from(value) * share + ground * (1.0 - share)).round() as u8;
        Color::new(channel(rgb.0), channel(rgb.1), channel(rgb.2), 255)
    };
    let fade = if enabled { 1.0 } else { 0.55 };
    if dark {
        RunLook {
            top: mix(0.50 * fade),
            bottom: mix(0.28 * fade),
            mid: mix(0.39 * fade),
            edge: mix(0.70 * fade),
        }
    } else {
        RunLook {
            top: mix(0.45 * fade),
            bottom: mix(0.62 * fade),
            mid: mix(0.53 * fade),
            edge: mix(0.85 * fade),
        }
    }
}

fn scene_rgb(scene: SceneId) -> (u8, u8, u8) {
    let colour = scene_color(scene, 255);
    (colour.r, colour.g, colour.b)
}

/// The run's poster in its slot, fading into the run across its right 40 %.
///
/// Drawn with a *positive* source height: the service flips each finished
/// poster to top row first (`posters::PosterView`), so it is an ordinary
/// texture here. `tools/poster_check.sh` checks it lands upright.
#[allow(
    clippy::too_many_arguments,
    reason = "one poster's draw: the texture, its shape, its slot, and the fade"
)]
fn draw_poster(
    d: &mut RaylibDrawHandle<'_>,
    poster: &raylib::prelude::RenderTexture2D,
    aspect: f32,
    rect: UiRect,
    slot_width: f32,
    opacity: f32,
    fade_into: Color,
) {
    let height = rect.height - 2.0;
    let width = (height * aspect.max(0.1)).min(slot_width);
    if width < 2.0 || height < 2.0 {
        return;
    }
    let texture: &raylib::ffi::Texture2D = poster.as_ref();
    let source_width = texture.width as f32;
    let source_height = texture.height as f32;
    // A slot narrower than the poster crops its right side rather than
    // squeezing it.
    let crop = (width / (height * aspect.max(0.1))).min(1.0);
    let dest = UiRect::new(rect.x + 1.0, rect.y + 1.0, width, height);
    d.draw_texture_pro(
        poster,
        raylib::prelude::Rectangle::new(0.0, 0.0, source_width * crop, source_height),
        widgets::rectangle(dest),
        Vector2::zero(),
        0.0,
        widgets::alpha(Color::WHITE, opacity),
    );
    let fade = UiRect::new(
        dest.x + dest.width * 0.6,
        dest.y,
        dest.width * 0.4,
        dest.height,
    );
    let clear = Color::new(fade_into.r, fade_into.g, fade_into.b, 0);
    d.draw_rectangle_gradient_ex(widgets::rectangle(fade), clear, clear, fade_into, fade_into);
}

#[cfg(test)]
fn packed(color: Color) -> u32 {
    u32::from_be_bytes([color.r, color.g, color.b, color.a])
}

/// `x,y,w,h` at whole logical pixels, as the report lines print a rect.
fn rect_fields(rect: UiRect) -> String {
    format!(
        "{:.0},{:.0},{:.0},{:.0}",
        rect.x, rect.y, rect.width, rect.height
    )
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

    /// The lane head (WX-5c §3): the plan's size and Auto on the left, the
    /// selected section in context in the middle, the event markers as chips
    /// on the right, in the one 26 px row the plan controls had. Placed by
    /// [`LaneHead::layout`], which sheds labels and words — never a gesture —
    /// as the band narrows.
    ///
    /// The widget ids are the controls row's (`TIMELINE` 20 + index) and the
    /// event row's (`EVENT_ROW`), so nothing a probe or a hand pressed before
    /// has moved namespace.
    fn scene_plan_controls(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        row: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        self.scene_lane.head_report.clear();
        let Some(track) = input.workspace.current() else {
            return;
        };
        let font = input.fonts.ui();
        let cues = track.scene_switches.cues();
        let selected = cues
            .iter()
            .enumerate()
            .find(|(_, cue)| cue.id == self.scene_lane.selected_id);
        let selected_scene =
            selected.and_then(|(_, cue)| SceneId::from_index(cue.scene_index as usize));

        let runs = fold_runs(cues).len();
        let label_full = if cues.is_empty() {
            "PLAN \u{00b7} NO SECTIONS".to_owned()
        } else {
            format!(
                "PLAN \u{00b7} {runs} RUN{} \u{00b7} {} SECTION{}",
                if runs == 1 { "" } else { "S" },
                cues.len(),
                if cues.len() == 1 { "" } else { "S" }
            )
        };
        let label_short = if cues.is_empty() {
            "PLAN".to_owned()
        } else {
            format!(
                "PLAN \u{00b7} {runs} RUN{}",
                if runs == 1 { "" } else { "S" }
            )
        };
        let caption = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
        let tracked = |text: &str| {
            widgets::measure(font, text, metric::UI_FONT_MICRO) + text.chars().count() as f32
        };
        let button = |text: &str| (caption(text) + 20.0).ceil();
        let hint_text = if track.scene_switches.finish.pixel.is_on() {
            let look = track.scene_switches.finish.pixel;
            format!(
                "click a section to edit it \u{00b7} pixel look {} {}",
                look.palette_label(),
                look.pixels
            )
        } else if track.scene_switches.finish.is_active() {
            "click a section to edit it \u{00b7} film finish on".to_owned()
        } else {
            "click a section to edit it".to_owned()
        };
        let face = track.manual_clear.button();
        let (markers_full, markers_short, clear_full, clear_short) =
            super::events::chip_widths(font, face);
        let widths = LaneHeadWidths {
            label_full: tracked(&label_full).ceil(),
            label_short: tracked(&label_short).ceil(),
            auto: button("Auto off").max(button("Auto on")),
            arrow: 26.0,
            name_min: 120.0,
            split_full: button("Split here"),
            split_short: button("Split"),
            capture_full: button("Capture tuning"),
            capture_short: button("Capture"),
            delete: button("Delete"),
            hint: (caption("click a section to edit it") + 4.0).ceil(),
            markers_full,
            markers_short,
            clear_full,
            clear_short,
        };
        let Some(head) = LaneHead::layout(row, &widths, selected.is_some()) else {
            self.scene_lane.head_report = "not drawn (no room)".to_owned();
            return;
        };

        if let Some(rect) = head.label {
            let text = if head.label_short {
                &label_short
            } else {
                &label_full
            };
            widgets::draw_text_tracked(
                d,
                font,
                text,
                rect.x,
                rect.y + (rect.height - metric::UI_FONT_MICRO) * 0.5,
                metric::UI_FONT_MICRO,
                1.0,
                color::ui_muted(),
            );
        }
        let gap = 0.0;
        let mut auto_x = head.auto.x;
        if control_button(
            self,
            d,
            input,
            head.auto,
            &mut auto_x,
            gap,
            0,
            head.auto.width,
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
        let mut split_x = head.split.x;
        if control_button(
            self,
            d,
            input,
            head.split,
            &mut split_x,
            gap,
            1,
            head.split.width,
            if head.split_short {
                "Split"
            } else {
                "Split here"
            },
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
        if let Some(rect) = head.hint {
            let text = fit_text(input, &hint_text, rect.width);
            widgets::draw_text(
                d,
                font,
                &text,
                rect.x + 4.0,
                rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }

        let mut report = vec![
            format!("step={}", head.step),
            format!(
                "selected={}",
                selected.map_or_else(|| "none".to_owned(), |(index, _)| (index + 1).to_string())
            ),
            format!("markers={}", track.manual_events.len()),
            format!("auto={}", rect_fields(head.auto)),
            format!("split={}", rect_fields(head.split)),
        ];

        if let (Some((index, cue)), Some(scene)) = (selected, selected_scene) {
            if let Some(rect) = head.prev {
                let mut x = rect.x;
                if control_button(
                    self,
                    d,
                    input,
                    rect,
                    &mut x,
                    gap,
                    2,
                    rect.width,
                    "<",
                    true,
                    false,
                    ButtonStyle::Neutral,
                )
                .clicked
                {
                    let previous =
                        SceneId::ALL[(scene.index() + SceneId::ALL.len() - 1) % SceneId::ALL.len()];
                    commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Retarget {
                        cue_id: cue.id,
                        scene: previous,
                    }));
                }
                report.push(format!("prev={}", rect_fields(rect)));
            }
            // The name carries what a section's colour cannot (MV9-k): which
            // section, which scene, where it starts, and then a threshold's
            // beyond, how it arrives, a glide, a grade.
            if let Some(rect) = head.name {
                let detail = scene_lane_edit::segment_summary(cue, &track.scene_settings);
                let mut text = format!(
                    "Section {} \u{00b7} {} \u{00b7} {}",
                    index + 1,
                    scene.display_name(),
                    widgets::format_timestamp(cue.start_seconds)
                );
                if let Some(detail) = detail {
                    text.push_str(" \u{00b7} ");
                    text.push_str(&detail);
                }
                let text = fit_text(input, &text, rect.width - 12.0);
                widgets::fill(d, rect, color::ui_raised());
                d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, color::ui_rule());
                widgets::draw_text(
                    d,
                    font,
                    &text,
                    rect.x + 6.0,
                    rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5,
                    metric::UI_FONT_CAPTION,
                    color::ui_ink(),
                );
                report.push(format!("name={}", rect_fields(rect)));
            }
            if let Some(rect) = head.next {
                let mut x = rect.x;
                if control_button(
                    self,
                    d,
                    input,
                    rect,
                    &mut x,
                    gap,
                    3,
                    rect.width,
                    ">",
                    true,
                    false,
                    ButtonStyle::Neutral,
                )
                .clicked
                {
                    let next = SceneId::ALL[(scene.index() + 1) % SceneId::ALL.len()];
                    commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Retarget {
                        cue_id: cue.id,
                        scene: next,
                    }));
                }
                report.push(format!("next={}", rect_fields(rect)));
            }
            if let Some(rect) = head.capture {
                let mut x = rect.x;
                let label = if head.capture_short {
                    "Capture"
                } else {
                    "Capture tuning"
                };
                if control_button(
                    self,
                    d,
                    input,
                    rect,
                    &mut x,
                    gap,
                    4,
                    rect.width,
                    label,
                    true,
                    false,
                    ButtonStyle::Neutral,
                )
                .clicked
                {
                    commands.push(ShellCommand::ScenePlan(ScenePlanEdit::CaptureTuning {
                        cue_id: cue.id,
                    }));
                }
                report.push(format!("capture={}", rect_fields(rect)));
            }
            if let Some(rect) = head.delete {
                let mut x = rect.x;
                if control_button(
                    self,
                    d,
                    input,
                    rect,
                    &mut x,
                    gap,
                    5,
                    rect.width,
                    "Delete",
                    true,
                    false,
                    ButtonStyle::Danger,
                )
                .clicked
                {
                    commands.push(ShellCommand::ScenePlan(ScenePlanEdit::Remove {
                        cue_id: cue.id,
                    }));
                }
                report.push(format!("delete={}", rect_fields(rect)));
            }
        }

        if let (Some(markers), Some(clear)) = (head.markers, head.clear) {
            let mut actions = Vec::new();
            self.event_chips(
                d,
                input,
                markers,
                clear,
                (head.markers_short, head.clear_short),
                &mut actions,
            );
            commands.extend(actions.into_iter().map(ShellCommand::ManualEvent));
            for (name, rect) in ["feel", "scene", "custom"].into_iter().zip(markers) {
                report.push(format!("{name}={}", rect_fields(rect)));
            }
            report.push(format!("clear={}", rect_fields(clear)));
        }
        self.scene_lane.head_report = report.join(" ");
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
        let cues = track.scene_switches.cues();
        let enabled = track.scene_switches.enabled;
        // Every section's span with a boundary drag's preview applied, so a run
        // stretches under the hand the way its sections do.
        let bounds: Vec<(f64, f64)> = cues
            .iter()
            .enumerate()
            .map(|(index, cue)| {
                let mut span = (cue.start_seconds, cue.end_seconds);
                if let Some(drag) = drag {
                    if cue.id == drag.right_cue_id {
                        span.0 = drag.preview_seconds;
                    }
                    if cues
                        .get(index + 1)
                        .is_some_and(|next| next.id == drag.right_cue_id)
                    {
                        span.1 = drag.preview_seconds;
                    }
                }
                span
            })
            .collect();
        // `--ui-probe run-open=N`, once there is a plan to open a run of.
        // N counts from 1, as the `scene runs:` line's `now=run N/M` does.
        if let Some(number) = self.probe_run_open.take() {
            let runs = fold_runs(cues);
            if let Some(run) = number.checked_sub(1).and_then(|index| runs.get(index)) {
                self.open_run(run, track.duration_seconds);
            }
        }
        // The section and run under the playhead wear the accent (WX-5a: the
        // accent means *now*), read off the same playhead the lane draws.
        let playhead = self.timeline_playhead_seconds(input.time_seconds);
        let x_of = |seconds: f64| {
            self.timeline
                .x_at(seconds, f64::from(lane.x), f64::from(lane.width)) as f32
        };
        let runs = fold_runs(cues);
        let mut stats = RunFrame {
            runs: runs.len(),
            sections: cues.len(),
            arriving: cues.iter().filter(|cue| !cue.transition.is_cut()).count(),
            graded: cues
                .iter()
                .filter(|cue| cue.grade.is_some_and(|grade| !grade.is_identity()))
                .count(),
            selected: cues
                .iter()
                .position(|cue| cue.id == self.scene_lane.selected_id),
            opened: self.scene_lane.opened,
            lane,
            view: (
                self.timeline.start_seconds,
                self.timeline.start_seconds + self.timeline.span_seconds,
            ),
            ..RunFrame::default()
        };
        let track_slot = input.workspace.current_index();
        for (run_index, run) in runs.iter().enumerate() {
            let last = run.first_section + run.sections - 1;
            let (start, end) = (bounds[run.first_section].0, bounds[last].1);
            stats.cuts += run.internal_cuts();
            let geometry = timed_lane::block_geometry(
                &self.timeline,
                f64::from(lane.x),
                f64::from(lane.width),
                start,
                end,
            );
            if let Some(slot) = track_slot {
                self.poster_requests.push(crate::posters::PosterRequest {
                    track_slot: slot,
                    run: *run,
                    visible: geometry.is_some(),
                });
            }
            let Some(block) = geometry else {
                continue;
            };
            // One pixel of the lane between two runs, as the mockup's
            // `margin-right: 1px`: a run edge is a scene change, a hairline is
            // not, and the two must not look alike.
            let rect = UiRect::new(
                block.left as f32,
                lane.y + 2.0,
                (block.width() as f32 - 1.0).max(1.0),
                lane.height - 4.0,
            );
            let look = run_look(scene_rgb(run.scene), theme::palette(), enabled);
            d.draw_rectangle_gradient_ex(
                widgets::rectangle(rect),
                look.top,
                look.bottom,
                look.bottom,
                look.top,
            );

            // The poster at the left edge, fading into the run (§3 of the WX-5b
            // note). Its slot is reserved whether or not the picture has
            // arrived, so the name does not jump when it does.
            let mut text_left = rect.x + 6.0;
            let mut poster_note: Option<&'static str> = None;
            if let Some(slot) = track_slot {
                let slot_width = (rect.height * 16.0 / 9.0).min(rect.width * 0.6);
                let poster = input.posters.view(track, slot, run);
                let drawn = match poster {
                    crate::posters::PosterView::Ready { texture, aspect } => {
                        Some((texture, aspect, 1.0))
                    }
                    crate::posters::PosterView::Stale { texture, aspect } => {
                        Some((texture, aspect, 0.45))
                    }
                    crate::posters::PosterView::Pending => None,
                    crate::posters::PosterView::Unavailable(reason) => {
                        stats.unavailable += 1;
                        poster_note = Some(reason);
                        None
                    }
                };
                if !matches!(poster, crate::posters::PosterView::Unavailable(_)) {
                    text_left = rect.x + slot_width + 6.0;
                }
                if let Some((texture, aspect, opacity)) = drawn {
                    stats.posters_ready += usize::from(opacity >= 1.0);
                    draw_poster(d, texture, aspect, rect, slot_width, opacity, look.mid);
                }
            }
            stats.posters_total += usize::from(track_slot.is_some());

            // Per section: the transition edge, glide ramp, grade bar and pixel
            // mark, each in that section's own span, so a grade change inside a
            // run is a change in the bar under it.
            for index in run.first_section..=last {
                let cue = &cues[index];
                let (from, to) = (x_of(bounds[index].0), x_of(bounds[index].1));
                let left = from.max(rect.x);
                let right = to.min(rect.x + rect.width);
                if right - left < 1.0 {
                    continue;
                }
                let section = UiRect::new(left, rect.y, right - left, rect.height);
                draw_segment_marks(
                    d,
                    &self.timeline,
                    lane,
                    section,
                    cue,
                    bounds[index].0,
                    if enabled { 255 } else { 92 },
                );
            }
            // A hairline at every cut inside the run: opaque and whole pixels,
            // so it is the same colour on every renderer and the gate can count
            // it exactly.
            for &(cut, _) in &bounds[run.first_section + 1..=last] {
                let x = x_of(cut).round();
                if x > rect.x + 1.0 && x < rect.x + rect.width - 1.0 {
                    widgets::fill(
                        d,
                        UiRect::new(x, rect.y, 1.0, rect.height),
                        color::ui_run_cut(),
                    );
                    stats.hairlines += 1;
                }
            }
            d.draw_rectangle_lines_ex(widgets::rectangle(rect), 1.0, look.edge);

            // The scene's name, if it fits beside the poster. It sits on the
            // run's fill, but internal-cut hairlines, transition edges and a
            // pixel mark all cross that fill, so the name carries a one-pixel
            // shadow in the panel's own tone (dark under light ink, light under
            // dark ink). A shadow rather than a plate: an opaque plate would
            // blank the hairlines' columns under the name, and the gate counts
            // hairlines by exact colour down the whole run body.
            let label = run.scene.display_name();
            let label_width = widgets::measure(input.fonts.ui(), label, metric::UI_FONT_CAPTION);
            if text_left + label_width + 6.0 <= rect.x + rect.width {
                let label_y = rect.y + rect.height - metric::UI_FONT_CAPTION - 7.0;
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    label,
                    text_left + 1.0,
                    label_y + 1.0,
                    metric::UI_FONT_CAPTION,
                    widgets::alpha(color::ui_surface(), 0.8),
                );
                widgets::draw_text(
                    d,
                    input.fonts.ui(),
                    label,
                    text_left,
                    label_y,
                    metric::UI_FONT_CAPTION,
                    color::ui_ink(),
                );
            }

            // The selected section, as an ink outline over its span inside the
            // run: selection is an outline, never the accent.
            for index in run.first_section..=last {
                if cues[index].id != self.scene_lane.selected_id {
                    continue;
                }
                let left = x_of(bounds[index].0).max(rect.x);
                let right = x_of(bounds[index].1).min(rect.x + rect.width);
                if right - left >= 2.0 {
                    d.draw_rectangle_lines_ex(
                        widgets::rectangle(UiRect::new(left, rect.y, right - left, rect.height)),
                        2.0,
                        color::ui_ink(),
                    );
                }
            }

            // What the run is, in words, under the pointer: its scene, its
            // sections and its span, how to open it, and why it has no poster
            // when the service refused one. Never claims a press.
            let pointer = self.widgets.pointer(d);
            let hovered = self.timeline_gesture.is_none()
                && self.scene_lane.drag.is_none()
                && rect.contains_point(pointer.x, pointer.y);
            let tip = format!(
                "{} - sections {}-{} - {}-{} - double-click to open{}",
                run.scene.display_name(),
                run.first_section + 1,
                last + 1,
                widgets::format_timestamp(start),
                widgets::format_timestamp(end),
                poster_note.map_or_else(String::new, |note| format!(" ({note})")),
            );
            self.widgets.hint(
                d,
                widgets::ButtonState {
                    hovered,
                    ..widgets::ButtonState::default()
                },
                widgets::widget_id(widgets::id::TIMELINE, RUN_HINT_IDS + run_index as u32),
                rect,
                &tip,
            );

            // Now: the run under the playhead gets an inset accent outline, and
            // the section under it a 2 px accent tick along the run's top.
            if enabled && run.contains(playhead) && rect.width > 4.0 {
                d.draw_rectangle_lines_ex(
                    widgets::rectangle(UiRect::new(
                        rect.x + 1.0,
                        rect.y + 1.0,
                        rect.width - 2.0,
                        rect.height - 2.0,
                    )),
                    1.0,
                    color::accent(),
                );
                for (index, &(from, to)) in bounds
                    .iter()
                    .enumerate()
                    .take(last + 1)
                    .skip(run.first_section)
                {
                    if from <= playhead && playhead < to {
                        let left = x_of(from).max(rect.x + 1.0);
                        let right = x_of(to).min(rect.x + rect.width - 1.0);
                        if right > left {
                            widgets::fill(
                                d,
                                UiRect::new(left, rect.y + 1.0, right - left, 2.0),
                                color::accent(),
                            );
                        }
                        stats.now = Some((run_index, index, run.scene));
                    }
                }
            }
        }
        self.scene_lane.last_frame = stats;
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

    /// Opens a run: the view zooms to it with a margin either side (WX-5b, the
    /// mockup's "double-click a run to open its sections"). At that zoom its
    /// sections are wide enough to aim at, and Zoom out or the wheel undo it.
    /// Detaches playback-follow, as any manual view change does.
    fn open_run(&mut self, run: &RunSpan, duration_seconds: f64) {
        let span = run.end_seconds - run.start_seconds;
        if !(span.is_finite() && span > 0.0) {
            return;
        }
        let margin = span * OPEN_RUN_MARGIN;
        self.timeline.start_seconds = run.start_seconds - margin;
        self.timeline.span_seconds = span + margin * 2.0;
        self.timeline.clamp(duration_seconds);
        self.timeline_manual_view = true;
        self.scene_lane.opened = Some(run.first_cue_id);
    }

    fn scene_lane_gesture(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        lane: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        // Through the widget bank's pointer seam rather than raylib directly
        // (WX-5b), so `--ui-probe click=` can press the lane the way it presses
        // a button. With no probe installed it is the same four reads.
        let pointer = self.widgets.pointer(d);
        let mouse = Vector2::new(pointer.x, pointer.y);
        if self.timeline_gesture == Some(TimelineGesture::Scrub) {
            self.scene_lane.scrub_origin_x = None;
            self.update_timeline_scrub(
                mouse.x,
                lane,
                track.duration_seconds,
                pointer.down,
                commands,
            );
            return;
        }
        if let Some(origin_x) = self.scene_lane.scrub_origin_x {
            if !pointer.down {
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
                || !pointer.pressed
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
                self.scene_lane.last_press = None;
                return;
            };
            self.scene_lane.selected_id = hit.id;
            // A second press on a section body inside the double-click window
            // opens the run under it. Boundaries are excluded: two quick presses
            // on a cut are two drags, not an instruction to zoom.
            let now = d.get_time();
            let double = hit.zone != SceneLaneZone::Boundary
                && is_double_click(self.scene_lane.last_press, now, mouse.x);
            if double {
                self.scene_lane.last_press = None;
                let seconds = self.timeline.seconds_at(
                    f64::from(mouse.x),
                    f64::from(lane.x),
                    f64::from(lane.width),
                    track.duration_seconds,
                );
                let runs = fold_runs(track.scene_switches.cues());
                if let Some(run) = runs.iter().find(|run| run.contains(seconds)) {
                    self.open_run(run, track.duration_seconds);
                }
                return;
            }
            self.scene_lane.last_press =
                (hit.zone != SceneLaneZone::Boundary).then_some((now, mouse.x));
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
        if pointer.down {
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

    /// A run's name is ink on the run's own fill (WX-5b), in both appearances,
    /// at both ends of the fill's gradient, for every scene — and muted on a
    /// disabled plan's fill. Measured rather than chosen, because the fill is a
    /// mix of a content colour nobody here controls (`ui/scene_colors.rs`). A
    /// disabled plan keeps ink names: its fill sinking toward the ground is the
    /// signal, and muted grey on a light disabled run measured 4.33:1.
    #[test]
    fn every_run_name_reads_on_its_run_in_both_appearances() {
        for scene in SceneId::ALL {
            for palette in [&palette::DARK, &palette::LIGHT] {
                for enabled in [true, false] {
                    let ink = palette.ui_ink;
                    let look = run_look(scene_rgb(scene), palette, enabled);
                    for fill in [look.top, look.bottom] {
                        let ratio = contrast::ratio(ink, packed(fill));
                        assert!(
                            ratio >= contrast::AA_TEXT,
                            "{} on {:06X} (enabled {enabled}): {ratio:.2}",
                            scene.display_name(),
                            palette.ui_surface >> 8
                        );
                    }
                }
            }
        }
    }

    /// The negative control: the 5a lane drew the scene colour at full
    /// strength, and ink-light names on it fail for the brightest scenes —
    /// which is why the run mixes toward the ground at all.
    #[test]
    fn full_strength_scene_colours_would_fail_the_run_name_rule() {
        let fluid = scene_rgb(SceneId::FluidInk);
        let full = packed(Color::new(fluid.0, fluid.1, fluid.2, 255));
        assert!(contrast::ratio(palette::DARK.ui_ink, full) < contrast::AA_TEXT);
    }

    #[test]
    fn a_double_click_is_two_presses_close_in_time_and_place() {
        let first = Some((10.0, 200.0));
        assert!(is_double_click(first, 10.3, 202.0));
        // The controls: too late, too far, no first press, and a clock that
        // went backwards.
        assert!(!is_double_click(first, 10.5, 200.0));
        assert!(!is_double_click(first, 10.2, 210.0));
        assert!(!is_double_click(None, 10.2, 200.0));
        assert!(!is_double_click(first, 9.9, 200.0));
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
