//! Shared mapping-editor componentry (UX0-C14).
//!
//! The Tune inspector's route editor and the caption effects' drive tuning are
//! the same interaction — a quiet/loud input window mapped to an output range
//! through a curve, with a live meter proving what the music is doing to it —
//! so the pieces both draw live here, parameterized on plain values rather
//! than on either owner's state. The route editor keeps its
//! `route_editor_state` machine and descriptors; the caption pane writes a
//! `DriveTuning` straight into the style. Neither can drift from the other's
//! *look* without this file changing, which is the point.
//!
//! Everything here is stateless between frames: callers pass values in and get
//! edits back.

use musializer_core::scene::routes::Interpolation;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{RaylibDraw, RaylibDrawHandle, Vector2};

use musializer_runtime::font::UiFonts;

use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle, Widgets};

/// `→`, or `->` on the fallback face.
///
/// raylib's default face stops at ASCII 126 and draws U+2192 as a missing-glyph
/// box. The interface face has it, so the substitution is reachable only when
/// that face failed to load. Takes `loaded` rather than the face itself so both
/// substitutions are testable without a window.
#[must_use]
pub fn arrow(loaded: bool) -> &'static str {
    if loaded {
        "\u{2192}"
    } else {
        "->"
    }
}

/// Where `live` sits inside the `[input_min, input_max]` window, as a 0..=1
/// meter fill (`scene_route_meter`, `plug.c:5574-5590`).
#[must_use]
pub fn window_position(input_min: f64, input_max: f64, live: f64) -> f32 {
    let span = input_max - input_min;
    if !(live.is_finite() && span.is_finite()) || span <= 0.0 {
        return 0.0;
    }
    (((live - input_min) / span) as f32).clamp(0.0, 1.0)
}

/// The live-source meter bar: a rule-coloured track with an accent fill.
pub fn meter(d: &mut RaylibDrawHandle<'_>, bar: UiRect, fill: f32) {
    widgets::fill(d, bar, color::ui_rule());
    widgets::fill(
        d,
        UiRect::new(bar.x, bar.y, bar.width * fill.clamp(0.0, 1.0), bar.height),
        color::accent(),
    );
}

/// One anchor: a label line above an input slider and an output slider joined
/// by an arrow (`plug.c:5723-5776` — the pairing *is* the layout; grouping the
/// four sliders by axis hid the diagonal input→output relationship).
pub struct AnchorPair<'a> {
    /// "Quiet  0.20 → 0.40" — built by the caller, whose precision it is.
    pub caption: &'a str,
    /// Both 0..=1; the caller owns the mapping to real values.
    pub input_fraction: f32,
    pub output_fraction: f32,
    pub input_id: u64,
    pub output_id: u64,
    pub arrow_ok: bool,
}

/// What an anchor row's sliders reported this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnchorEdit {
    pub input: Option<f32>,
    pub output: Option<f32>,
}

/// Width of the arrow gutter between an anchor's two sliders.
pub const ARROW_WIDTH: f32 = 18.0;
const GAP: f32 = 8.0;

/// Draws one anchor row (caption at `y`, sliders 15 px below, 20 px tall) and
/// returns any slider movement. Total height: 40 px — the route editor's
/// stride, which the caption pane matches rather than re-deciding.
pub fn anchor_pair(
    d: &mut RaylibDrawHandle<'_>,
    widgets_state: &mut Widgets,
    font: &UiFonts,
    area_x: f32,
    y: f32,
    width: f32,
    pair: &AnchorPair<'_>,
) -> AnchorEdit {
    widgets::draw_text(d, font, pair.caption, area_x, y, 12.0, color::ui_muted());
    let pair_width = (width - ARROW_WIDTH - GAP * 2.0) * 0.5;
    let input_slider = UiRect::new(area_x, y + 15.0, pair_width, 20.0);
    let output_slider = UiRect::new(
        area_x + pair_width + ARROW_WIDTH + GAP * 2.0,
        y + 15.0,
        pair_width,
        20.0,
    );
    let glyph = arrow(pair.arrow_ok);
    let glyph_width = widgets::measure(font, glyph, 14.0);
    widgets::draw_text(
        d,
        font,
        glyph,
        area_x + pair_width + GAP + (ARROW_WIDTH - glyph_width) * 0.5,
        y + 15.0 + (20.0 - 14.0) * 0.5,
        14.0,
        color::ui_muted(),
    );
    AnchorEdit {
        input: widgets_state.slider(d, pair.input_id, input_slider, pair.input_fraction),
        output: widgets_state.slider(d, pair.output_id, output_slider, pair.output_fraction),
    }
}

/// The `<  Curve: Linear  >` stepper row (`plug.c:5784-5808`). Returns the
/// newly chosen curve when a stepper was clicked.
#[allow(clippy::too_many_arguments)]
pub fn curve_stepper(
    d: &mut RaylibDrawHandle<'_>,
    widgets_state: &mut Widgets,
    font: &UiFonts,
    area_x: f32,
    y: f32,
    width: f32,
    previous_id: u64,
    next_id: u64,
    current: Interpolation,
) -> Option<Interpolation> {
    let curve_previous = UiRect::new(area_x, y, 34.0, 22.0);
    let curve_next = UiRect::new(area_x + width - 34.0, y, 34.0, 22.0);
    let curves = Interpolation::ALL.len();
    let position = Interpolation::ALL
        .iter()
        .position(|curve| *curve == current)
        .unwrap_or(0);
    let mut chosen = None;
    for (id, step, label, boundary) in [
        (previous_id, curves - 1, "<", curve_previous),
        (next_id, 1, ">", curve_next),
    ] {
        if widgets_state
            .text_button(
                d,
                font,
                id,
                boundary,
                label,
                false,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            )
            .clicked
        {
            chosen = Some(Interpolation::ALL[(position + step) % curves]);
        }
    }
    let caption = format!("Curve: {}", curve_label(current));
    let caption_width = widgets::measure(font, &caption, 13.0);
    widgets::draw_text(
        d,
        font,
        &caption,
        area_x + (width - caption_width) * 0.5,
        y + (22.0 - 13.0) * 0.5,
        13.0,
        color::ui_ink(),
    );
    chosen
}

/// [`musializer_core::ui::route_editor_state::curve_label`], re-exported so the
/// caption pane does not import a route-editor module for one string.
#[must_use]
pub fn curve_label(curve: Interpolation) -> &'static str {
    musializer_core::ui::route_editor_state::curve_label(curve)
}

/// The transfer curve: source level across, mapped value up
/// (`scene_route_transfer_graph`, `plug.c:5592-5641`).
///
/// `sample` must be the *same* function the frame loop maps with — the route
/// editor passes `ParameterMapping::output_value` (curve, swap, clamping and
/// toggle quantization included), the caption pane passes `DriveTuning::apply`
/// — so the graph draws exactly what will play. `floor`/`span` normalize the
/// sampled value onto the plot's vertical axis; the shaded band is the input
/// window; the dot rides the curve at the live value.
pub fn transfer_graph(
    d: &mut RaylibDrawHandle<'_>,
    plot: UiRect,
    input_window: (f64, f64),
    floor: f64,
    span: f64,
    live: Option<f64>,
    sample: &dyn Fn(f64) -> Option<f64>,
) {
    if plot.is_empty() {
        return;
    }
    let span = if span <= 0.0 { 1.0 } else { span };
    widgets::fill(d, plot, color::ui_raised());
    let window_left = input_window.0.clamp(0.0, 1.0) as f32;
    let window_right = input_window.1.clamp(0.0, 1.0) as f32;
    if window_right > window_left {
        let mut shade = color::accent();
        shade.a = (0.12 * 255.0) as u8;
        widgets::fill(
            d,
            UiRect::new(
                plot.x + plot.width * window_left,
                plot.y,
                plot.width * (window_right - window_left),
                plot.height,
            ),
            shade,
        );
    }
    d.draw_rectangle_lines_ex(widgets::rectangle(plot), 1.0, color::ui_rule());

    const SAMPLES: i32 = 64;
    let mut previous: Option<Vector2> = None;
    for step in 0..=SAMPLES {
        let source = f64::from(step) / f64::from(SAMPLES);
        let Some(value) = sample(source) else {
            previous = None;
            continue;
        };
        let point = Vector2::new(
            plot.x + plot.width * source as f32,
            plot.y + plot.height * (1.0 - ((value - floor) / span) as f32),
        );
        if let Some(from) = previous {
            d.draw_line_ex(from, point, 2.0, color::accent());
        }
        previous = Some(point);
    }

    if let Some(live) = live {
        if let Some(mapped) = sample(live) {
            let dot = Vector2::new(
                plot.x + plot.width * live.clamp(0.0, 1.0) as f32,
                plot.y + plot.height * (1.0 - ((mapped - floor) / span) as f32),
            );
            d.draw_circle_v(dot, 4.0, color::ui_ink());
            d.draw_circle_v(dot, 2.5, color::accent());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_position_matches_the_route_editors() {
        assert_eq!(window_position(0.0, 1.0, 0.25), 0.25);
        assert_eq!(window_position(0.2, 0.7, 0.7), 1.0);
        assert_eq!(window_position(0.2, 0.7, 0.0), 0.0);
        assert_eq!(window_position(0.5, 0.5, 0.5), 0.0, "degenerate window");
        assert_eq!(window_position(0.0, 1.0, f64::NAN), 0.0);
    }

    #[test]
    fn the_arrow_falls_back_to_ascii() {
        assert_eq!(arrow(true), "\u{2192}");
        assert_eq!(arrow(false), "->");
    }
}
