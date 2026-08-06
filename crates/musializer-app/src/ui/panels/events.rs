//! The manual event row, and the shared preset controls.
//!
//! **Owner: Agent L.** C sources: `plug.c:2834-2971` (the row), `:5979-6100`
//! (the preset block), `event_timeline.c`, `semantic_lane.c`, `preset_store.c`.
//!
//! Both surfaces are drawing only. Every decision they make — which face the
//! `Clear manual` button shows, what a marker record contains, where the preset
//! selection lands after a delete — lives in
//! [`musializer_core::project::event_timeline`] and
//! [`musializer_core::project::preset_store`], where it is tested against the
//! `.c` rather than against a screenshot.
//!
//! # The oracle's honesty, kept
//!
//! **A manual marker carries one value.**
//! [`musializer_core::project::semantic_lane::sample`] requires the four-value
//! analysis payload and skips anything else, so a `+ Feel` marker never reaches
//! the semantic lane at all — only Constellation's generic event path reads it.
//! The C says so in a tooltip (`plug.c:2839-2844`); this row says so in a caption
//! that is always on screen, because this rewrite has no tooltip widget and
//! because a claim only a hover can reveal is a claim no capture can review.
//!
//! # Two things this row does *not* reproduce
//!
//! - **The `Lyrics` / `Assist` / `Export` buttons.** The C seats them in this row
//!   (`plug.c:2830`); this rewrite seats them in the toolbar. Drawing them in
//!   both places would put two controls in charge of one open panel, which is
//!   exactly the defect the C's own comment at `plug.c:2884-2887` records.
//! - **`ShellCommand`.** The row returns [`ManualEventAction`]s and the preset
//!   block returns [`PresetAction`]s, because a leaf agent may not add a
//!   `ShellCommand` variant. The call sites wrap them; see this session's NOTE
//!   ENTRIES for the two-variant diff that makes them ordinary commands.

use musializer_core::project::event_timeline::{manual_marker, ClearButton, ManualEventAction};
use musializer_core::project::preset_store::{self, PresetAction, SharedPresetsView};
use musializer_core::scene::events::EventType;
use musializer_core::scene::settings::PRESETS_PER_SCENE;
use musializer_core::ui::notice::Severity;
use musializer_core::ui::timeline_layout::TimelineBand;
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle};

use super::super::shell::{Shell, ShellInput};
use super::super::theme::{color, metric};
use super::super::widgets::{self, ButtonStyle};

/// Widget id namespaces for the two surfaces in this file.
///
/// These were local literals — `7` and `8` — chosen to "continue the same
/// sequence" while the fan-out was live, with a comment promising they would be
/// moved into [`widgets::id`] at merge. They were not, and by then `7` was
/// `EXPORT` and `8` was `SEEK`. Both surfaces draw *before* their twins, so this
/// row silently ate the export panel's 720p/1080p/1440p buttons and the transport
/// row silently ate four of the preset picker's (EX1). Aliases now, so the value
/// lives once, in the table the collision test enumerates.
const EVENT_ROW_NAMESPACE: u32 = widgets::id::EVENT_ROW;
const PRESET_NAMESPACE: u32 = widgets::id::EVENT_PRESET;

/// The oracle's control-row geometry (`plug.c:2822-2861`).
///
/// `#[rustfmt::skip]` because these are checked against the C column by column,
/// and one width per line stops being checkable.
#[rustfmt::skip]
mod geometry {
    /// `controls_height` (`plug.c:2822`).
    pub const CONTROLS_HEIGHT: f32 = 38.0;
    /// `margin` (`plug.c:2824`).
    pub const MARGIN: f32 = 6.0;
    /// The three marker buttons' natural widths, from the tail of the C's
    /// `control_widths` (`plug.c:2853`): `+ Feel`, `+ Scene`, `+ Custom`.
    pub const MARKER_WIDTHS: [f32; 3] = [74.0, 82.0, 86.0];
    /// The trailing `Clear manual` slot (`plug.c:2870`).
    pub const CLEAR_WIDTH: f32 = 112.0;
    /// Gap above the always-visible honesty caption, which the oracle keeps in a
    /// tooltip instead.
    pub const CAPTION_GAP: f32 = 3.0;
}

/// The height [`Shell::event_row`] takes when it draws at all.
///
/// A constant rather than only a return value, because the caller has to *reserve*
/// it before the row is measured — [`Shell::timeline_height`] is asked how tall
/// the strip should be before anything inside it is laid out. The row still
/// returns what it actually took, and returns `0.0` when it drew nothing, so a
/// reservation and a use can never silently disagree.
pub(crate) const EVENT_ROW_HEIGHT: f32 = geometry::MARGIN
    + geometry::CONTROLS_HEIGHT
    + geometry::CAPTION_GAP
    + metric::UI_FONT_CAPTION
    + geometry::MARGIN;

/// The three markers the row records, in the C's order (`plug.c:2830`, `:2856`).
///
/// `+ Scene` is not an event at all — it cues the scene switch plan — which is
/// why it carries no [`EventType`].
#[rustfmt::skip]
const MARKERS: [(&str, Option<EventType>); 3] = [
    ("+ Feel",   Some(EventType::Semantic)),
    ("+ Scene",  None),
    ("+ Custom", Some(EventType::Custom)),
];

/// The colour a marker button's category strip draws in.
///
/// `+ Scene` borrows [`EventType::Cue`]'s, which is what the C does: its
/// `types[4]` is `EVENT_TYPE_CUE` even though the button records a scene switch
/// rather than an event (`plug.c:2857`).
fn marker_color(kind: Option<EventType>) -> Color {
    Color::get_color(kind.unwrap_or(EventType::Cue).rgba())
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::new(
        color.r,
        color.g,
        color.b,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

impl Shell {
    /// The manual event row, above the timeline's waveform lane.
    ///
    /// Returns **the height it consumed**, which is the layout rule this
    /// repository has already paid for: a row only some modes have is a
    /// parameter, never an assumption. A row that cannot seat its own controls —
    /// no track open, too little height, or a band too narrow even at
    /// [`musializer_core::ui::timeline_layout::TIMELINE_BAND_MIN_SCALE`] —
    /// consumes nothing and draws nothing, rather than claiming clicks it cannot
    /// show a control for (`workspace_layout.h:7-19`).
    // No call site yet: its caller is `shell.rs`, which a leaf agent does not
    // edit. The wiring diff is in this session's NOTE ENTRIES.
    #[allow(dead_code)]
    pub(crate) fn event_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        actions: &mut Vec<ManualEventAction>,
    ) -> f32 {
        // Not drawn over the lyrics editor: see `Shell::timeline_height` for why
        // the band cannot afford both, and why that is the oracle's arithmetic
        // rather than a preference.
        if matches!(
            self.panel,
            crate::cli::UiPanel::Lyrics | crate::cli::UiPanel::Assist
        ) {
            return 0.0;
        }
        use geometry::{CAPTION_GAP, CLEAR_WIDTH, CONTROLS_HEIGHT, MARGIN, MARKER_WIDTHS};

        let Some(track) = input.workspace.current() else {
            return 0.0;
        };
        let consumed = EVENT_ROW_HEIGHT;
        if area.width <= 0.0 || area.height < consumed {
            return 0.0;
        }

        // The timecode is already placed by the toolbar or the timeline header,
        // so this band asks for none — declaring a width it does not draw would
        // shrink the row for a string that is not there.
        let Some(band) = TimelineBand::layout(
            area.x,
            area.y + MARGIN,
            area.width,
            CONTROLS_HEIGHT,
            MARGIN,
            &MARKER_WIDTHS,
            CLEAR_WIDTH,
            0.0,
        ) else {
            return 0.0;
        };
        if !band.fits {
            // Even the minimum scale overflows. The band reports that rather than
            // hiding it, and the answer here is to draw nothing: the timeline
            // below is worth more than three buttons at an illegible size.
            return 0.0;
        }

        let font = input.fonts.ui();
        let scaled: Vec<f32> = MARKER_WIDTHS
            .iter()
            .map(|width| width * band.scale)
            .collect();
        let labels: Vec<&str> = MARKERS.iter().map(|(label, _)| *label).collect();
        // One shared label size for the row, so a longer label shrinks its
        // neighbours with it instead of rendering visibly smaller than them
        // (`ui_row_typography.h:9-13`).
        let font_size = widgets::row_font_size(font, &labels, &scaled, CONTROLS_HEIGHT);

        let mut cursor = band.controls.x;
        for (index, (label, kind)) in MARKERS.iter().enumerate() {
            let boundary = UiRect::new(cursor, band.controls.y, scaled[index], CONTROLS_HEIGHT);
            cursor += scaled[index] + MARGIN * band.scale;

            let id = widgets::widget_id(EVENT_ROW_NAMESPACE, index as u32);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                boundary,
                label,
                false,
                ButtonStyle::Neutral,
                Some(font_size),
            );
            // The category strip and outline (`plug.c:2860-2866`), which is what
            // tells three otherwise identical buttons apart at a glance.
            let category = marker_color(*kind);
            widgets::fill(
                d,
                UiRect::new(boundary.x, boundary.y, 4.0, boundary.height),
                category,
            );
            d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 2.0, with_alpha(category, 0.8));

            if !state.clicked {
                continue;
            }
            match kind {
                None => actions.push(ManualEventAction::RecordSceneCue),
                Some(kind) => {
                    match manual_marker(input.time_seconds, track.next_manual_event_id, *kind) {
                        Some(record) => actions.push(ManualEventAction::Record(record)),
                        // `plug.c:1961-1965`. Unreachable short of 2^64 markers,
                        // and said out loud anyway: a button that silently does
                        // nothing is worse than one that explains itself.
                        None => self.notify(
                            Severity::Error,
                            "Manual event was not added",
                            "The event ID space is exhausted.",
                        ),
                    }
                }
            }
        }

        // `Clear manual` / `Confirm clear` / `Undo clear` (`plug.c:2852-2855`).
        let face = track.manual_clear.button();
        let clear = UiRect::new(
            band.clear.x,
            band.clear.y,
            band.clear.width,
            CONTROLS_HEIGHT,
        );
        let clear_id = widgets::widget_id(EVENT_ROW_NAMESPACE, 100);
        // Nothing recorded and nothing to put back means nothing to do. Drawn
        // disabled rather than hidden, so the control still names what it is for.
        let live = face != ClearButton::Clear || !track.manual_events.is_empty();
        if live {
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    clear_id,
                    clear,
                    face.label(),
                    face.is_danger(),
                    if face.is_danger() {
                        ButtonStyle::Danger
                    } else {
                        ButtonStyle::Neutral
                    },
                    None,
                )
                .clicked
            {
                let action = face.click();
                if action == ManualEventAction::ArmClear {
                    // The C pushes the warning together with the confirmation, so
                    // the second click is never a surprise (`plug.c:2966-2969`).
                    self.notify(
                        Severity::Warning,
                        "Confirm manual-event clear",
                        "Click Confirm clear. Lyrics and imported lanes will remain.",
                    );
                }
                actions.push(action);
            }
        } else {
            self.widgets
                .disabled_button(d, font, clear, face.label(), None);
        }

        // The honesty caption, and the marker count beside it. The count is what
        // a headless capture can assert on: it distinguishes "the row drew" from
        // "the row recorded".
        let markers = track.manual_events.len();
        let caption = format!(
            "{markers} manual marker{}  -  a marker carries one value, so only Constellation reacts",
            if markers == 1 { "" } else { "s" }
        );
        widgets::draw_text(
            d,
            font,
            &caption,
            band.controls.x,
            band.controls.y + CONTROLS_HEIGHT + CAPTION_GAP,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        consumed
    }

    /// The shared preset block, inside the tuning inspector
    /// (`scene_settings_panel`'s preset zone, `plug.c:5979-6100`).
    ///
    /// **Not a panel of its own** — that misreading has cost this repository a
    /// session before. It is a strip between the inspector's header buttons and
    /// its first slider, and like [`Self::event_row`] it returns the height it
    /// consumed so the list below starts where the block actually ended.
    ///
    /// Two heights, both the oracle's: 42 px when the scene has no presets and
    /// 98 px when it has. The collapsed state exists because the full block cost
    /// 98 px of a panel whose first slider is what the user opened it for, and
    /// with nothing saved only `Save new` did anything (`plug.c:5995-5999`).
    // No call site yet: its caller is `tune.rs`, another agent's file.
    #[allow(dead_code)]
    pub(crate) fn preset_block(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        presets: SharedPresetsView<'_>,
        actions: &mut Vec<PresetAction>,
    ) -> f32 {
        let font = input.fonts.ui();
        let padding = metric::UI_PANEL_PADDING;
        let left = area.x + padding;
        let right = area.x + area.width - padding;
        if right - left <= 0.0 {
            return 0.0;
        }

        let scene_presets = presets.library.presets(input.scene);
        let count = scene_presets.len();
        let selected = preset_store::clamp_selection(count, presets.selected);
        let status = format!("PRESETS  {count} / {PRESETS_PER_SCENE}");

        if count == 0 {
            let consumed = 42.0;
            if area.height < consumed {
                return 0.0;
            }
            widgets::draw_text(
                d,
                font,
                &status,
                left,
                area.y + 9.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            let save = UiRect::new(right - 96.0, area.y, 96.0, 30.0);
            self.preset_save_button(d, input, save, &presets, count, None, actions);
            return consumed;
        }

        let consumed = 98.0;
        if area.height < consumed {
            return 0.0;
        }
        widgets::draw_text(
            d,
            font,
            &status,
            left,
            area.y + 3.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );

        // The pager. `<` and `>` wrap, and the name between them is a disabled
        // button rather than plain text so the selection reads as a control
        // (`plug.c:6013-6031`).
        let small_gap = 5.0f32;
        let nav_y = area.y + 22.0;
        let previous = UiRect::new(left, nav_y, 34.0, 28.0);
        let next = UiRect::new(right - 34.0, nav_y, 34.0, 28.0);
        let name = UiRect::new(
            previous.x + previous.width + small_gap,
            nav_y,
            (next.x - previous.x - previous.width - small_gap * 2.0).max(0.0),
            28.0,
        );
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 0),
                previous,
                "<",
                false,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            actions.push(PresetAction::Select(preset_store::step_selection(
                count, selected, false,
            )));
        }
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 1),
                next,
                ">",
                false,
                ButtonStyle::Neutral,
                None,
            )
            .clicked
        {
            actions.push(PresetAction::Select(preset_store::step_selection(
                count, selected, true,
            )));
        }
        self.widgets
            .disabled_button(d, font, name, &scene_presets[selected].name, None);

        // Load / Save / Update / Delete, four equal cells (`plug.c:6033-6046`).
        let action_y = nav_y + 34.0;
        let action_width = ((right - left) - small_gap * 3.0) * 0.25;
        let cell = |index: usize| {
            UiRect::new(
                left + (action_width + small_gap) * index as f32,
                action_y,
                action_width,
                30.0,
            )
        };
        let delete_label = if presets.delete_armed {
            "Confirm"
        } else {
            "Delete"
        };
        let labels = [
            "Load",
            preset_store::save_label(count),
            "Update",
            delete_label,
        ];
        let widths = [action_width; 4];
        let action_font = widgets::row_font_size(font, &labels, &widths, 30.0);

        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 2),
                cell(0),
                labels[0],
                false,
                ButtonStyle::Neutral,
                Some(action_font),
            )
            .clicked
        {
            actions.push(PresetAction::Apply(selected));
        }

        self.preset_save_button(
            d,
            input,
            cell(1),
            &presets,
            count,
            Some(action_font),
            actions,
        );

        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 3),
                cell(2),
                labels[2],
                false,
                ButtonStyle::Neutral,
                Some(action_font),
            )
            .clicked
        {
            self.request_preset_write(&presets, PresetAction::Replace(selected), actions);
        }

        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 4),
                cell(3),
                delete_label,
                presets.delete_armed,
                if presets.delete_armed {
                    ButtonStyle::Danger
                } else {
                    ButtonStyle::Neutral
                },
                Some(action_font),
            )
            .clicked
        {
            if presets.delete_armed {
                self.request_preset_write(&presets, PresetAction::Delete(selected), actions);
            } else {
                // Arming is not a write, so it is offered even on a read-only
                // store — and the refusal then lands on the click that would
                // actually have changed something.
                actions.push(PresetAction::ArmDelete(selected));
            }
        }

        consumed
    }

    /// `Save` / `Save new` / `Full`, shared by the collapsed and the populated
    /// block exactly as `scene_settings_save_new_preset` is (`plug.c:4226`).
    #[allow(
        clippy::too_many_arguments,
        reason = "draw target, input, box, view, count, size, sink — every one is a distinct input"
    )]
    fn preset_save_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        boundary: UiRect,
        presets: &SharedPresetsView<'_>,
        count: usize,
        font_size: Option<f32>,
        actions: &mut Vec<PresetAction>,
    ) {
        let font = input.fonts.ui();
        // The collapsed state has room for the long label; the quarter-width cell
        // does not, which is why the C shortens it there (`plug.c:6044-6046`).
        let label = if count == 0 {
            "Save new"
        } else {
            preset_store::save_label(count)
        };
        if count >= PRESETS_PER_SCENE {
            self.widgets
                .disabled_button(d, font, boundary, label, font_size);
            return;
        }
        if self
            .widgets
            .text_button(
                d,
                font,
                widgets::widget_id(PRESET_NAMESPACE, 5),
                boundary,
                label,
                false,
                ButtonStyle::Neutral,
                font_size,
            )
            .clicked
        {
            self.request_preset_write(presets, PresetAction::SaveNew, actions);
        }
    }

    /// Emits a preset mutation, or explains why it will not
    /// (`shared_presets_editable`, `plug.c:4200-4209`).
    ///
    /// A store file that was rejected at startup stays **read-only**: overwriting
    /// it would destroy something the user could still fix by hand, and this is
    /// the only place that decision is visible to them.
    fn request_preset_write(
        &mut self,
        presets: &SharedPresetsView<'_>,
        action: PresetAction,
        actions: &mut Vec<PresetAction>,
    ) {
        if presets.editable {
            actions.push(action);
        } else {
            self.notify(
                Severity::Warning,
                "Shared presets are read-only",
                "Fix or remove the preset library file, then restart Musializer.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This file's two namespaces are entries in the shared table, not literals.
    ///
    /// The predecessor of this test enumerated a hand-written six-name list that
    /// happened to omit `EXPORT` and `SEEK` — the two this file had actually
    /// collided with — and passed for as long as the defect existed. So it now
    /// asserts *membership* rather than non-collision: the completeness of the
    /// table is `widgets`' own test to keep, and there is one table.
    #[test]
    fn the_two_namespaces_are_allocated_in_the_shared_table() {
        let allocated: Vec<u32> = widgets::id::ALL.iter().map(|(_, value)| *value).collect();
        assert!(allocated.contains(&EVENT_ROW_NAMESPACE));
        assert!(allocated.contains(&PRESET_NAMESPACE));
        assert_ne!(EVENT_ROW_NAMESPACE, PRESET_NAMESPACE);
    }

    #[test]
    fn the_marker_row_names_the_three_the_oracle_records() {
        // The tail of `plug.c:2830`, and `:2856-2858`'s types. `+ Scene` carries
        // no event type because it cues the switch plan rather than the lane.
        assert_eq!(MARKERS[0].0, "+ Feel");
        assert_eq!(MARKERS[0].1, Some(EventType::Semantic));
        assert_eq!(MARKERS[1].0, "+ Scene");
        assert_eq!(MARKERS[1].1, None);
        assert_eq!(MARKERS[2].0, "+ Custom");
        assert_eq!(MARKERS[2].1, Some(EventType::Custom));
    }

    #[test]
    fn the_row_band_seats_its_controls_at_the_narrowest_supported_timeline() {
        use geometry::{CLEAR_WIDTH, CONTROLS_HEIGHT, MARGIN, MARKER_WIDTHS};
        // The 960 px minimum window with the inspector open is the case
        // `timeline_layout.h:9-21` names, and the timeline keeps roughly 620 px
        // of it. The row has to seat itself there or it is not drawn at all.
        for width in [620.0f32, 700.0, 960.0, 1280.0, 1920.0] {
            let band = TimelineBand::layout(
                0.0,
                0.0,
                width,
                CONTROLS_HEIGHT,
                MARGIN,
                &MARKER_WIDTHS,
                CLEAR_WIDTH,
                0.0,
            )
            .expect("a legal band");
            assert!(band.fits, "the marker row must fit at {width} px");
            assert!(
                band.clear.x + band.clear.width <= width,
                "the clear button runs past the band at {width} px"
            );
        }
    }
}
