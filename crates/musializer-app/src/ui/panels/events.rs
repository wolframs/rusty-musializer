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

/// Saved looks in Look's column (WX-5d §3.3): the preset block in the column's
/// vocabulary. Chips, one per preset, each a press that loads it; `+ Save this
/// look`; and, for the selected chip, Update and Delete at the right end of the
/// section head. Every action the oracle's block had is here: `<`/`>` became
/// pressing a chip, `Load` became the same press.
pub(crate) struct SavedLooks {
    pub labels: Vec<String>,
    pub chip_widths: Vec<f32>,
    pub save_label: &'static str,
    pub save_width: f32,
    /// The empty library's sentence, and its width; empty when there are presets.
    pub hint: &'static str,
    pub hint_width: f32,
    /// Update and Delete (or Confirm), present only with a selection.
    pub action_labels: Vec<&'static str>,
    pub action_widths: Vec<f32>,
    pub selected: Option<usize>,
}

/// Padding either side of a chip's label.
const LOOK_CHIP_PADDING: f32 = 12.0;
/// Padding either side of a head action's label.
const HEAD_ACTION_PADDING: f32 = 8.0;
/// Chip ids in [`PRESET_NAMESPACE`], after the oracle block's six.
const CHIP_SLOT: u32 = 10;

impl SavedLooks {
    /// What the column will draw for `scene`, measured in `font`.
    pub(crate) fn measure(
        font: &musializer_runtime::font::UiFonts,
        presets: &SharedPresetsView<'_>,
        scene: musializer_core::scene::SceneId,
    ) -> Self {
        let size = metric::UI_FONT_CAPTION;
        let scene_presets = presets.library.presets(scene);
        let count = scene_presets.len();
        let labels: Vec<String> = scene_presets
            .iter()
            .map(|preset| preset.name.clone())
            .collect();
        let chip_widths = labels
            .iter()
            .map(|label| (widgets::measure(font, label, size) + LOOK_CHIP_PADDING * 2.0).ceil())
            .collect();
        let save_label = if count >= PRESETS_PER_SCENE {
            "Library full"
        } else {
            "+ Save this look"
        };
        let hint = if count == 0 {
            "Saved looks follow you to every project."
        } else {
            ""
        };
        let selected = (count > 0).then(|| preset_store::clamp_selection(count, presets.selected));
        let action_labels: Vec<&'static str> = if selected.is_some() {
            vec![
                "Update",
                if presets.delete_armed {
                    "Confirm delete"
                } else {
                    "Delete"
                },
            ]
        } else {
            Vec::new()
        };
        Self {
            chip_widths,
            save_width: (widgets::measure(font, save_label, size) + LOOK_CHIP_PADDING * 2.0).ceil(),
            hint_width: if hint.is_empty() {
                0.0
            } else {
                widgets::measure(font, hint, size).ceil()
            },
            action_widths: action_labels
                .iter()
                .map(|label| {
                    (widgets::measure(font, label, size) + HEAD_ACTION_PADDING * 2.0).ceil()
                })
                .collect(),
            labels,
            save_label,
            hint,
            action_labels,
            selected,
        }
    }
}

impl Shell {
    /// Draws Look's saved looks at the rects `tune_groups::look_layout` gave
    /// them and returns what was pressed, as the oracle block's actions.
    #[allow(
        clippy::too_many_arguments,
        reason = "the measured content, every rect the layout produced, the view and the sink"
    )]
    pub(crate) fn saved_looks(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        looks: &SavedLooks,
        head: UiRect,
        action_rects: &[UiRect],
        chips: &[UiRect],
        save: UiRect,
        hint: Option<UiRect>,
        actions: &mut Vec<PresetAction>,
    ) {
        let font = input.fonts.ui();
        let presets = input.presets;
        let count = looks.labels.len();
        let count_text = format!("{count}");
        let total_text = format!("{PRESETS_PER_SCENE}");
        widgets::section_head(
            d,
            font,
            UiRect::new(
                head.x,
                head.y,
                action_rects
                    .first()
                    .map_or(head.width, |first| first.x - 8.0 - head.x),
                head.height,
            ),
            &[
                widgets::MicroRun::muted("Saved looks · "),
                widgets::MicroRun::ink(&count_text),
                widgets::MicroRun::muted(" / "),
                widgets::MicroRun::muted(&total_text),
            ],
        );

        for (index, (rect, label)) in chips.iter().zip(&looks.labels).enumerate() {
            let selected = looks.selected == Some(index);
            let id = widgets::widget_id(PRESET_NAMESPACE, CHIP_SLOT + index as u32);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                *rect,
                label,
                selected,
                ButtonStyle::Neutral,
                Some(metric::UI_FONT_CAPTION),
            );
            if state.clicked {
                // One press is the oracle's `<`/`>` and `Load` together: the
                // chip a user presses is the look they want to see.
                actions.push(PresetAction::Select(index));
                actions.push(PresetAction::Apply(index));
            }
            self.widgets.hint(
                d,
                state,
                id,
                *rect,
                "Load this look. Revert on the trial card brings back what you had.",
            );
        }

        if count >= PRESETS_PER_SCENE {
            self.widgets.disabled_button(
                d,
                font,
                save,
                looks.save_label,
                Some(metric::UI_FONT_CAPTION),
            );
        } else {
            let id = widgets::widget_id(PRESET_NAMESPACE, 5);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                save,
                looks.save_label,
                false,
                ButtonStyle::Ghost,
                Some(metric::UI_FONT_CAPTION),
            );
            if !state.hovered {
                // Ghost draws nothing idle; a dashed-looking hairline would be
                // noise, a plain one says "this is a place to put something".
                widgets::control_box(d, save, color::ui_surface(), color::ui_rule(), 1.0);
                widgets::text_label(
                    d,
                    font,
                    save,
                    looks.save_label,
                    metric::UI_FONT_CAPTION,
                    color::ui_ink(),
                );
            }
            if state.clicked {
                self.request_preset_write(&presets, PresetAction::SaveNew, actions);
            }
            self.widgets.hint(
                d,
                state,
                id,
                save,
                "Save these values as a look for this scene, in every project",
            );
        }
        if let Some(hint) = hint {
            widgets::draw_text_faded(
                d,
                font,
                looks.hint,
                hint.x,
                hint.y + (hint.height - metric::UI_FONT_CAPTION) * 0.5,
                hint.width,
                metric::UI_FONT_CAPTION + 4.0,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
        }

        let Some(selected) = looks.selected else {
            return;
        };
        for (slot, (rect, label)) in action_rects.iter().zip(&looks.action_labels).enumerate() {
            let delete = slot == 1;
            let id = widgets::widget_id(PRESET_NAMESPACE, if delete { 4 } else { 3 });
            let state = self.widgets.button(d, id, *rect);
            if state.hovered {
                widgets::control_box(d, *rect, color::ui_hover(), color::ui_hover(), 1.0);
            }
            widgets::text_label(
                d,
                font,
                *rect,
                label,
                metric::UI_FONT_CAPTION,
                if delete {
                    color::ui_danger()
                } else if state.hovered {
                    color::ui_ink()
                } else {
                    color::ui_muted()
                },
            );
            if state.clicked {
                if !delete {
                    self.request_preset_write(&presets, PresetAction::Replace(selected), actions);
                } else if presets.delete_armed {
                    self.request_preset_write(&presets, PresetAction::Delete(selected), actions);
                } else {
                    actions.push(PresetAction::ArmDelete(selected));
                }
            }
            self.widgets.hint(
                d,
                state,
                id,
                *rect,
                if delete {
                    "Delete the selected look; a second press confirms"
                } else {
                    "Replace the selected look with the values on screen"
                },
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
    fn the_marker_chips_name_the_three_the_oracle_records() {
        // The tail of `plug.c:2830`, and `:2856-2858`'s types. `+ Scene` carries
        // no event type because it cues the switch plan rather than the lane.
        assert_eq!(MARKERS[0].0, "+ Feel");
        assert_eq!(MARKERS[0].2, Some(EventType::Semantic));
        assert_eq!(MARKERS[1].0, "+ Scene");
        assert_eq!(MARKERS[1].2, None);
        assert_eq!(MARKERS[2].0, "+ Custom");
        assert_eq!(MARKERS[2].2, Some(EventType::Custom));
    }
}
