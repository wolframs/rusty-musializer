//! The lane head: one 26 px row above the plan lane (WX-5c, `docs/WX5C_DESIGN.md`
//! §3).
//!
//! **Not a port.** It replaces two rows the band used to carry — the manual
//! event row (66 px: `+ Feel`, `+ Scene`, `+ Custom`, `Clear manual` and a
//! caption) and the scene plan's controls row (26 px: Auto, Split here, the
//! retarget arrows, the section's name, Capture tuning, Delete) — with one row
//! of the second's height. The band is 66 px shorter in every state that drew
//! the event row, and the preview has those pixels.
//!
//! Left to right: the plan's size and the Auto toggle; the section in context
//! (with one selected: its arrows, its name, Split here, Capture tuning,
//! Delete; with none: Split here and a hint); the event markers as chips on
//! the right.
//!
//! # The compaction ladder
//!
//! A narrow band cannot seat all of that, and the alternative to shedding is a
//! row of controls squeezed until they are unreadable. So the row is chosen
//! from a ladder of whole configurations, each **cumulative** step taking width
//! away and none giving it back, which makes the choice monotone in width by
//! construction — the property `transport_bar` learned the hard way, where a
//! greedy placer made a control vanish and return as the window narrowed:
//!
//! 1. the plan label loses its section count;
//! 2. `Capture tuning` becomes `Capture`;
//! 3. `Clear manual` becomes `Clear`;
//! 4. `Split here` becomes `Split`;
//! 5. the marker chips drop their `+`;
//! 6. the plan label goes;
//! 7. the section's name (or, with none selected, the hint) goes;
//! 8. the marker chips go — they come back when the band widens, as the old
//!    event row did, which drew nothing at all below its own minimum.
//!
//! Auto, Split, the arrows and Delete are never shed: they are the plan's
//! editing gestures, and the lane under them depends on them.
//!
//! Pure, and measured by the caller: this module has no font, so every width
//! is handed in, which is what lets the ladder be swept headlessly.

use super::workspace_layout::UiRect;

/// The row's height: the plan controls row it replaces.
pub const LANE_HEAD_HEIGHT: f32 = 26.0;
/// Between two controls of one group.
pub const GAP: f32 = 6.0;
/// Between the three groups.
pub const GROUP_GAP: f32 = 16.0;
/// The last rung. Past it nothing more can be given up.
pub const LAST_STEP: usize = 8;

/// What each control costs, measured by the caller at the size it draws.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LaneHeadWidths {
    /// `PLAN · 27 RUNS · 70 SECTIONS`, and the same without the sections.
    pub label_full: f32,
    pub label_short: f32,
    pub auto: f32,
    /// The retarget arrows either side of the name.
    pub arrow: f32,
    /// The least a section's name is worth drawing at: below it the name is
    /// dropped rather than ellipsized to a stub.
    pub name_min: f32,
    pub split_full: f32,
    pub split_short: f32,
    pub capture_full: f32,
    pub capture_short: f32,
    pub delete: f32,
    /// "click a section to edit it", with nothing selected.
    pub hint: f32,
    pub markers_full: [f32; 3],
    pub markers_short: [f32; 3],
    pub clear_full: f32,
    pub clear_short: f32,
}

/// Where each control went, or `None` for one this rung shed or this state
/// does not draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LaneHead {
    /// How far down the ladder the row had to go: 0 is everything at full
    /// width, [`LAST_STEP`] is the plan's editing controls alone.
    pub step: usize,
    pub label: Option<UiRect>,
    pub label_short: bool,
    pub auto: UiRect,
    pub prev: Option<UiRect>,
    pub name: Option<UiRect>,
    pub next: Option<UiRect>,
    pub split: UiRect,
    pub split_short: bool,
    pub capture: Option<UiRect>,
    pub capture_short: bool,
    pub delete: Option<UiRect>,
    pub hint: Option<UiRect>,
    /// `+ Feel`, `+ Scene`, `+ Custom`.
    pub markers: Option<[UiRect; 3]>,
    pub markers_short: bool,
    pub clear: Option<UiRect>,
    pub clear_short: bool,
}

/// The shape one rung asks for, before it is placed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rung {
    label: Option<f32>,
    label_short: bool,
    split: f32,
    split_short: bool,
    capture: f32,
    capture_short: bool,
    /// The flexible field — the name, or the hint — and its least width,
    /// `None` once it is shed.
    flex: Option<f32>,
    markers: Option<[f32; 3]>,
    markers_short: bool,
    clear: f32,
    clear_short: bool,
}

impl Rung {
    fn at(step: usize, widths: &LaneHeadWidths, selected: bool) -> Self {
        let label_short = step >= 1;
        let capture_short = step >= 2;
        let clear_short = step >= 3;
        let split_short = step >= 4;
        let markers_short = step >= 5;
        Rung {
            label: (step < 6).then_some(if label_short {
                widths.label_short
            } else {
                widths.label_full
            }),
            label_short,
            split: if split_short {
                widths.split_short
            } else {
                widths.split_full
            },
            split_short,
            capture: if capture_short {
                widths.capture_short
            } else {
                widths.capture_full
            },
            capture_short,
            flex: (step < 7).then_some(if selected {
                widths.name_min
            } else {
                widths.hint
            }),
            markers: (step < 8).then_some(if markers_short {
                widths.markers_short
            } else {
                widths.markers_full
            }),
            markers_short,
            clear: if clear_short {
                widths.clear_short
            } else {
                widths.clear_full
            },
            clear_short,
        }
    }

    fn left(&self, widths: &LaneHeadWidths) -> f32 {
        self.label.map_or(0.0, |label| label + GAP) + widths.auto
    }

    fn right(&self) -> f32 {
        self.markers.map_or(0.0, |markers| {
            markers.iter().sum::<f32>() + self.clear + GAP * 3.0
        })
    }

    /// The middle group at its least: the flexible field at its floor.
    fn middle(&self, widths: &LaneHeadWidths, selected: bool) -> f32 {
        let flex = self.flex.map_or(0.0, |flex| flex + GAP);
        if selected {
            widths.arrow * 2.0 + self.split + self.capture + widths.delete + GAP * 4.0 + flex
        } else {
            self.split + flex
        }
    }

    fn width(&self, widths: &LaneHeadWidths, selected: bool) -> f32 {
        let groups = if self.markers.is_some() { 2.0 } else { 1.0 };
        self.left(widths) + self.middle(widths, selected) + self.right() + GROUP_GAP * groups
    }
}

impl LaneHead {
    /// The richest rung that fits `row`, placed. `None` when even
    /// [`LAST_STEP`] does not, or for a degenerate row.
    #[must_use]
    pub fn layout(row: UiRect, widths: &LaneHeadWidths, selected: bool) -> Option<Self> {
        if !row.is_finite() || row.is_empty() {
            return None;
        }
        let (step, rung) = (0..=LAST_STEP)
            .map(|step| (step, Rung::at(step, widths, selected)))
            .find(|(_, rung)| rung.width(widths, selected) <= row.width)?;
        let mut head = LaneHead {
            step,
            label_short: rung.label_short,
            split_short: rung.split_short,
            capture_short: rung.capture_short,
            markers_short: rung.markers_short,
            clear_short: rung.clear_short,
            ..LaneHead::default()
        };
        let at = |x: f32, width: f32| UiRect::new(x, row.y, width, row.height);

        // Left, from the row's left edge.
        let mut x = row.x;
        if let Some(label) = rung.label {
            head.label = Some(at(x, label));
            x += label + GAP;
        }
        head.auto = at(x, widths.auto);
        x += widths.auto + GROUP_GAP;
        let middle_left = x;

        // Right, from the row's right edge.
        let mut right = row.x + row.width;
        let mut middle_right = right;
        if let Some(markers) = rung.markers {
            right -= rung.clear;
            head.clear = Some(at(right, rung.clear));
            let mut placed = [UiRect::default(); 3];
            for index in (0..3).rev() {
                right -= GAP + markers[index];
                placed[index] = at(right, markers[index]);
            }
            head.markers = Some(placed);
            middle_right = right - GROUP_GAP;
        }

        // The middle: the flexible field takes whatever the fixed controls
        // leave, which the rung guaranteed is at least its floor.
        let fixed = rung.middle(widths, selected) - rung.flex.map_or(0.0, |flex| flex);
        let flex = (middle_right - middle_left - fixed).max(0.0);
        let mut x = middle_left;
        if selected {
            head.prev = Some(at(x, widths.arrow));
            x += widths.arrow + GAP;
            if rung.flex.is_some() {
                head.name = Some(at(x, flex));
                x += flex + GAP;
            }
            head.next = Some(at(x, widths.arrow));
            x += widths.arrow + GAP;
            head.split = at(x, rung.split);
            x += rung.split + GAP;
            head.capture = Some(at(x, rung.capture));
            x += rung.capture + GAP;
            head.delete = Some(at(x, widths.delete));
        } else {
            head.split = at(x, rung.split);
            x += rung.split + GAP;
            if rung.flex.is_some() {
                head.hint = Some(at(x, flex));
            }
        }
        Some(head)
    }

    /// Every rect this layout placed, for overlap and containment checks.
    #[must_use]
    pub fn placed(&self) -> Vec<UiRect> {
        let mut rects = vec![self.auto, self.split];
        rects.extend(
            [
                self.label,
                self.prev,
                self.name,
                self.next,
                self.capture,
                self.delete,
                self.hint,
                self.clear,
            ]
            .into_iter()
            .flatten(),
        );
        if let Some(markers) = self.markers {
            rects.extend(markers);
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widths the shell measures at 100 %, rounded, so the arithmetic
    /// below is the arithmetic the window does.
    fn widths() -> LaneHeadWidths {
        LaneHeadWidths {
            label_full: 196.0,
            label_short: 92.0,
            auto: 68.0,
            arrow: 26.0,
            name_min: 120.0,
            split_full: 78.0,
            split_short: 48.0,
            capture_full: 104.0,
            capture_short: 64.0,
            delete: 60.0,
            hint: 160.0,
            markers_full: [60.0, 66.0, 72.0],
            markers_short: [48.0, 54.0, 60.0],
            clear_full: 96.0,
            clear_short: 52.0,
        }
    }

    fn row(width: f32) -> UiRect {
        UiRect::new(10.0, 600.0, width, LANE_HEAD_HEIGHT)
    }

    /// Pinned at the band widths the gate photographs: 1260 (1280 window) and
    /// 940 (960), with and without a selection.
    #[test]
    fn the_lane_head_at_the_photographed_widths() {
        let w = widths();
        let wide = LaneHead::layout(row(1260.0), &w, true).unwrap();
        assert_eq!(wide.step, 0);
        assert_eq!(wide.label, Some(UiRect::new(10.0, 600.0, 196.0, 26.0)));
        assert_eq!(wide.auto, UiRect::new(212.0, 600.0, 68.0, 26.0));
        assert_eq!(wide.prev, Some(UiRect::new(296.0, 600.0, 26.0, 26.0)));
        // 1270 right edge: clear 96, three chips and three gaps, a group gap.
        assert_eq!(wide.clear, Some(UiRect::new(1174.0, 600.0, 96.0, 26.0)));
        assert_eq!(
            wide.markers.map(|m| m[0]),
            Some(UiRect::new(958.0, 600.0, 60.0, 26.0))
        );
        // The name takes what is left: 942 - 296 - 26*2 - 78 - 104 - 60 - 6*5.
        assert_eq!(wide.name, Some(UiRect::new(328.0, 600.0, 322.0, 26.0)));
        assert_eq!(wide.delete, Some(UiRect::new(882.0, 600.0, 60.0, 26.0)));

        // 940 cannot afford the full label (954 at step 1) and takes the
        // second rung: the short label and `Capture`.
        let narrow = LaneHead::layout(row(940.0), &w, true).unwrap();
        assert_eq!(narrow.step, 2);
        assert!(narrow.label_short && narrow.capture_short && !narrow.clear_short);
        assert_eq!(narrow.name, Some(UiRect::new(224.0, 600.0, 146.0, 26.0)));

        let idle = LaneHead::layout(row(940.0), &w, false).unwrap();
        assert_eq!(idle.step, 0);
        assert_eq!(idle.split, UiRect::new(296.0, 600.0, 78.0, 26.0));
        assert_eq!(idle.hint, Some(UiRect::new(380.0, 600.0, 242.0, 26.0)));
        assert_eq!(idle.prev, None);
    }

    /// 960 with the inspector open leaves a 600 px band: the deepest rung the
    /// window can reach.
    #[test]
    fn the_narrowest_band_keeps_every_plan_gesture() {
        let head = LaneHead::layout(row(600.0), &widths(), true).unwrap();
        assert_eq!(head.step, 7);
        assert!(head.markers.is_some(), "the chips still fit at 600");
        assert_eq!(head.name, None);
        assert!(head.prev.is_some() && head.next.is_some() && head.delete.is_some());
        assert!(LaneHead::layout(row(100.0), &widths(), true).is_none());
    }

    #[test]
    fn the_ladder_is_monotone_and_nothing_overlaps_or_leaves_the_row() {
        for selected in [false, true] {
            let mut previous = 0usize;
            for width in (200..=2000).rev() {
                let row = row(width as f32);
                let Some(head) = LaneHead::layout(row, &widths(), selected) else {
                    continue;
                };
                assert!(
                    head.step >= previous,
                    "{width}: step {} after {previous}",
                    head.step
                );
                previous = head.step;
                let placed = head.placed();
                for (index, rect) in placed.iter().enumerate() {
                    assert!(
                        rect.x >= row.x - 0.01 && rect.x + rect.width <= row.x + row.width + 0.01,
                        "{width}/{selected}: control {index} leaves the row"
                    );
                    for other in &placed[index + 1..] {
                        assert!(!rect.overlaps(*other), "{width}/{selected}: overlap");
                    }
                }
            }
        }
    }
}
