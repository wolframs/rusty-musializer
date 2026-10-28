//! The manual event markers, and the shared preset controls.
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
//! The C says so in a tooltip (`plug.c:2839-2844`). This row said so in an
//! always-visible caption until WX-5c moved the markers onto the 26 px lane
//! head as chips; the sentence is their tooltip again, with the marker count
//! that the `lane head:` report line also carries.
//!
//! # What this does *not* reproduce
//!
//! - **The `Lyrics` / `Assist` / `Export` buttons.** The C seats them in this row
//!   (`plug.c:2830`); this rewrite seats them in the top bar's tabs. Drawing them
//!   in both places would put two controls in charge of one open panel, which is
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
use musializer_core::ui::workspace_layout::UiRect;
use raylib::prelude::{Color, RaylibDrawHandle};

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

/// The height the manual event row took from the band before WX-5c: a 38 px
/// control row between two 6 px margins, and a 13 px caption 3 px under it.
///
/// **No band spends it any more.** The row's controls are chips on the lane
/// head (`docs/WX5C_DESIGN.md` §3), and the caption is their tooltip. The
/// constant stays because `panels::export`'s band budget still adds it — and
/// subtracts it again through `EXPORT_PANEL_TIMELINE_HEIGHT`, which
/// `Shell::timeline_height` relies on — so removing it is that file's change
/// to make, and the shell takes it off the export minimum until then.
pub(crate) const EVENT_ROW_HEIGHT: f32 = 6.0 + 38.0 + 3.0 + metric::UI_FONT_CAPTION + 6.0;

/// The three markers the row records, in the C's order (`plug.c:2830`, `:2856`),
/// with the short label the lane head falls back to when the band is narrow.
///
/// `+ Scene` is not an event at all — it cues the scene switch plan — which is
/// why it carries no [`EventType`].
#[rustfmt::skip]
const MARKERS: [(&str, &str, Option<EventType>); 3] = [
    ("+ Feel",   "Feel",   Some(EventType::Semantic)),
    ("+ Scene",  "Scene",  None),
    ("+ Custom", "Custom", Some(EventType::Custom)),
];

/// Room a chip leaves for its category swatch and padding, beside its label.
const CHIP_PADDING: f32 = 26.0;

/// The colour a marker button's category strip draws in.
///
/// `+ Scene` borrows [`EventType::Cue`]'s, which is what the C does: its
/// `types[4]` is `EVENT_TYPE_CUE` even though the button records a scene switch
/// rather than an event (`plug.c:2857`).
fn marker_color(kind: Option<EventType>) -> Color {
    Color::get_color(kind.unwrap_or(EventType::Cue).rgba())
}

/// The short face of the clear control, for a narrow lane head.
fn clear_short(face: ClearButton) -> &'static str {
    match face {
        ClearButton::Clear => "Clear",
        ClearButton::Confirm => "Confirm",
        ClearButton::Undo => "Undo",
    }
}

/// The chips' widths at the caption size: the three markers full and short,
/// and the clear control full and short, for the lane head's ladder.
pub(crate) fn chip_widths(
    font: &musializer_runtime::font::UiFonts,
    face: ClearButton,
) -> ([f32; 3], [f32; 3], f32, f32) {
    let measure = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
    (
        MARKERS.map(|(full, _, _)| (measure(full) + CHIP_PADDING).ceil()),
        MARKERS.map(|(_, short, _)| (measure(short) + CHIP_PADDING).ceil()),
        (measure(face.label()) + 20.0).ceil(),
        (measure(clear_short(face)) + 20.0).ceil(),
    )
}

impl Shell {
    /// The manual event markers as chips on the lane head (WX-5c §3): `+ Feel`,
    /// `+ Scene`, `+ Custom` and the clear control, in rects the lane head
    /// placed. The widget ids are the event row's, so a probe or a hand that
    /// pressed `+ Feel` before presses the same control now.
    ///
    /// **The oracle's honesty, kept, one hover away.** A manual marker carries
    /// one value, so only Constellation reacts to it; the row said so in a
    /// caption that was always on screen, and the chips say it in their
    /// tooltip with the marker count, because the 26 px head has no line for
    /// a caption.
    pub(crate) fn event_chips(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        markers: [UiRect; 3],
        clear: UiRect,
        short: (bool, bool),
        actions: &mut Vec<ManualEventAction>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        let font = input.fonts.ui();
        let count = track.manual_events.len();
        let caption = format!(
            "{count} manual marker{} \u{00b7} a marker carries one value, so only Constellation reacts",
            if count == 1 { "" } else { "s" }
        );
        for (index, (full, brief, kind)) in MARKERS.iter().enumerate() {
            let boundary = markers[index];
            let id = widgets::widget_id(EVENT_ROW_NAMESPACE, index as u32);
            let label = if short.0 { brief } else { full };
            let state = self.widgets.button(d, id, boundary);
            let look = widgets::button_look(ButtonStyle::Neutral, false, state.hovered);
            let fill = if state.pressed {
                widgets::brightness(look.fill, -0.08)
            } else {
                look.fill
            };
            widgets::control_box(d, boundary, fill, look.edge, 1.0);
            // The category swatch (`plug.c:2860-2866`), which is what tells
            // three otherwise identical chips apart at a glance: a short bar
            // inside the rounded control, taller than it is wide, which is
            // what `tools/timeline_event_markers.py` tells it apart from a
            // marker head by.
            let swatch_height = (boundary.height * 0.5).round();
            widgets::fill(
                d,
                UiRect::new(
                    boundary.x + 8.0,
                    boundary.y + ((boundary.height - swatch_height) * 0.5).round(),
                    3.0,
                    swatch_height,
                ),
                marker_color(*kind),
            );
            widgets::draw_text(
                d,
                font,
                label,
                boundary.x + 17.0,
                boundary.y + (boundary.height - metric::UI_FONT_CAPTION) * 0.5,
                metric::UI_FONT_CAPTION,
                look.ink,
            );
            let tip = if kind.is_none() {
                "Cue a scene switch at the playhead".to_owned()
            } else {
                format!("Mark the playhead \u{00b7} {caption}")
            };
            self.widgets.hint(d, state, id, boundary, &tip);
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
        let label = if short.1 {
            clear_short(face)
        } else {
            face.label()
        };
        let clear_id = widgets::widget_id(EVENT_ROW_NAMESPACE, 100);
        // Nothing recorded and nothing to put back means nothing to do. Drawn
        // disabled rather than hidden, so the control still names what it is for.
        let live = face != ClearButton::Clear || !track.manual_events.is_empty();
        if !live {
            self.widgets
                .disabled_button(d, font, clear, label, Some(metric::UI_FONT_CAPTION));
            return;
        }
        let state = self.widgets.text_button(
            d,
            font,
            clear_id,
            clear,
            label,
            face.is_danger(),
            if face.is_danger() {
                ButtonStyle::Danger
            } else {
                ButtonStyle::Neutral
            },
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(
            d,
            state,
            clear_id,
            clear,
            &format!("Clear the manual markers \u{00b7} {caption}"),
        );
        if state.clicked {
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
            let consumed = preset_block_height(count);
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

        let consumed = preset_block_height(count);
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

/// Natural height shared by the drawing block and Tune's scroll measurement.
pub(crate) fn preset_block_height(count: usize) -> f32 {
    if count == 0 {
        42.0
    } else {
        98.0
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
    fn the_marker_chips_name_the_three_the_oracle_records() {
        // The tail of `plug.c:2830`, and `:2856-2858`'s types. `+ Scene` carries
        // no event type because it cues the switch plan rather than the lane.
        assert_eq!(MARKERS[0].0, "+ Feel");
        assert_eq!(MARKERS[0].2, Some(EventType::Semantic));
        assert_eq!(MARKERS[1].0, "+ Scene");
        assert_eq!(MARKERS[1].2, None);
        assert_eq!(MARKERS[2].0, "+ Custom");
        assert_eq!(MARKERS[2].2, Some(EventType::Custom));
        // The band the export budget still counts: 6 + 38 + 3 + 13 + 6.
        assert_eq!(EVENT_ROW_HEIGHT, 66.0);
    }
}
