//! The Lyrics view in the right column (WX-5d §3.2): Cues, Style, Effects and
//! the font browser, behind `Placement::Column`.
//!
//! **Owner: the Lyrics builder.** A child of [`super`] so it shares the
//! editor's private state and the band's helpers (the time readout, the free
//! picker, the tune editor, the draft guard) instead of copying them; a second
//! implementation of "commit a typed time" is a second place to get it wrong.
//! Every rect comes from `core::ui::lyrics_column_layout`, and what was drawn is
//! written to [`ColumnEvidence`] for the `lyrics column:` and `caption style:`
//! report lines, so a probe aims at a line and never at a pixel measured once.
//!
//! The cue lane stays in the band (the shell's `lyrics_panel` call); this file
//! draws only the column.

use musializer_core::project::caption_effects::drive_value;
use musializer_core::project::model::caption_fx;
use musializer_core::ui::lyrics_column_layout::{
    self as layout, CuesSpec, EffectsSpec, LabelledRow, PickerAt, PickerLayout, StyleSpec,
    TuneLayout,
};
use musializer_runtime::font::{Faces, Icon};
use raylib::prelude::RaylibFont;

use super::super::super::widgets::{MicroRun, SegmentedFit, SliderRow, StatusTone};
use super::*;

/// Which of the view's panes is showing (§3.2): one value read off the
/// editor's three flags, never a second copy of them. The flags stay because
/// `panels::assist` writes them (Look & Assist's file); folding them into this
/// enum is the merge's follow-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LyricsPane {
    Cues,
    Style,
    Effects,
    /// The font browser, a sub-state of Style: its head says `‹ Style`.
    Fonts,
}

impl LyricsPane {
    pub(crate) fn name(self) -> &'static str {
        match self {
            LyricsPane::Cues => "cues",
            LyricsPane::Style => "style",
            LyricsPane::Effects => "effects",
            LyricsPane::Fonts => "fonts",
        }
    }

    /// Whether the caption is what the pane shapes, so the preview should show
    /// one (the sample caption).
    pub(crate) fn shapes_caption(self) -> bool {
        self != LyricsPane::Cues
    }
}

impl LyricEditor {
    /// The pane, read off the flags with the band's precedence: the browser
    /// wins, then Effects, then Style.
    pub(crate) fn pane(&self) -> LyricsPane {
        match (self.style_pane, self.font_pane, self.effects_pane) {
            (false, _, _) => LyricsPane::Cues,
            (true, true, _) => LyricsPane::Fonts,
            (true, false, true) => LyricsPane::Effects,
            (true, false, false) => LyricsPane::Style,
        }
    }

    /// Writes the flags [`Self::pane`] reads. Leaving a pane closes its
    /// disclosures, as the band's toggles do: a picker or tune editor left open
    /// would write into a control no longer on screen.
    pub(crate) fn set_pane(&mut self, pane: LyricsPane) {
        let before = self.pane();
        self.style_pane = pane != LyricsPane::Cues;
        self.effects_pane = pane == LyricsPane::Effects;
        self.font_pane = pane == LyricsPane::Fonts;
        if before != pane {
            self.picker.target = None;
            self.tune_target = None;
            self.column.scroll = 0.0;
        }
        if pane != LyricsPane::Cues {
            self.text.set_focused(false);
            self.time_edit = None;
        }
    }
}

/// What the column drew in its last frame, for the report lines. Written while
/// drawing, because the report is printed at shutdown.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ColumnEvidence {
    /// The `lyrics column:` line's body.
    pub lyrics_line: String,
    /// The `caption style:` line's body; empty unless Style or Effects drew.
    pub style_line: String,
    /// The Style/Effects pane's scroll offset.
    pub scroll: f32,
}

/// Indices in [`ns::ACTIONS`] the column adds; the band uses 0..=8.
mod action {
    pub const PANES: u32 = 20;
    pub const BACK_TO_STYLE: u32 = 24;
    pub const PREVIOUS: u32 = 25;
    pub const NEXT: u32 = 26;
    pub const FOLLOW: u32 = 27;
    pub const PLAYHEAD_WORD: u32 = 28;
}

/// Indices in [`ns::CAPTION`] the column adds, above the band's (which end at
/// [`caption_id::ZONE_HINTS`] plus a handful). Each slider row takes
/// [`widgets::SLIDER_ROW_IDS`].
mod column_id {
    pub const SIZE: u32 = 192;
    pub const WIDTH: u32 = 196;
    pub const BLUR: u32 = 200;
    pub const SHADE: u32 = 204;
    pub const ROUND: u32 = 208;
    pub const INSET: u32 = 212;
    pub const GLOW: u32 = 216;
    pub const RADIUS: u32 = 220;
    pub const DEPTH: u32 = 224;
    pub const SWEEP: u32 = 228;
    pub const INK_PLUS: u32 = 232;
    pub const PLATE_PLUS: u32 = 233;
    pub const IMPORT_CANCEL: u32 = 234;
}

/// `x,y,w,h` in whole logical pixels, the report lines' rect format.
pub(crate) fn rect_token(rect: UiRect) -> String {
    format!(
        "{},{},{},{}",
        rect.x.round(),
        rect.y.round(),
        rect.width.round(),
        rect.height.round()
    )
}

/// The grid cell's words, row-major from the top, for the place line and the
/// report.
const ANCHOR_WORDS: [&str; 9] = [
    "top left",
    "top centre",
    "top right",
    "middle left",
    "centre",
    "middle right",
    "bottom left",
    "bottom centre",
    "bottom right",
];

/// The anchor enum's index for a grid cell: the enum counts up from the bottom
/// row, the grid draws down from the top (`:886-902`, as in the band).
const fn anchor_value(cell: usize) -> usize {
    (2 - cell / 3) * 3 + cell % 3
}

impl Shell {
    /// The Lyrics view, drawn into the column (`Placement::Column`).
    pub(super) fn lyrics_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        column: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        editor.column.lyrics_line.clear();
        editor.column.style_line.clear();
        if column.is_empty() {
            return;
        }
        let font = input.fonts.ui();
        let count = input
            .workspace
            .current()
            .map_or(0, |track| track.lyrics.len());
        let count_text = count.to_string();
        let pane = editor.pane();

        // The head: `LYRICS · 83 CUES`, then the pane switch, or `‹ Style`
        // from the browser. The word "cues" goes first when the column is too
        // narrow for both, never the switch.
        let labels = ["Cues", "Style", "Effects"];
        let switch_width = if pane == LyricsPane::Fonts {
            86.0
        } else {
            widgets::segmented_intrinsic_width(font, &labels)
        };
        let full = [
            MicroRun::muted("Lyrics · "),
            MicroRun::ink(&count_text),
            MicroRun::muted(" cues"),
        ];
        let short = [MicroRun::muted("Lyrics · "), MicroRun::ink(&count_text)];
        let room = column.width
            - widgets::COLUMN_HEAD_PADDING_LEFT
            - widgets::COLUMN_HEAD_PADDING_RIGHT
            - widgets::COLUMN_HEAD_GAP
            - switch_width;
        let title: &[MicroRun<'_>] = if widgets::micro_width(font, &full) <= room {
            &full
        } else {
            &short
        };
        let head = widgets::column_head(d, font, column, title);
        let switch = head.trailing_right(switch_width, widgets::SEGMENTED_HEIGHT);
        let mut seg_report = String::new();
        if pane == LyricsPane::Fonts {
            let back = UiRect::new(switch.x, switch.y, switch.width, switch.height);
            if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::ACTIONS, action::BACK_TO_STYLE),
                    back,
                    "\u{2039} Style",
                    false,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
            {
                editor.set_pane(LyricsPane::Style);
            }
            seg_report = format!("back@{}", rect_token(back));
        } else {
            let selected = match pane {
                LyricsPane::Cues => 0,
                LyricsPane::Effects => 2,
                _ => 1,
            };
            let result = self.widgets.segmented(
                d,
                font,
                ns::ACTIONS,
                action::PANES,
                switch,
                &labels,
                Some(selected),
                SegmentedFit::Hug,
            );
            for (index, item) in result.layout.items.iter().enumerate() {
                if !seg_report.is_empty() {
                    seg_report.push(',');
                }
                let name = ["cues", "style", "effects"][index];
                let star = if index == selected { "*" } else { "" };
                seg_report.push_str(&format!("{name}{star}@{}", rect_token(*item)));
            }
            if let Some(chosen) = result.clicked.filter(|chosen| *chosen != selected) {
                let target = [LyricsPane::Cues, LyricsPane::Style, LyricsPane::Effects][chosen];
                // Leaving Cues hides the draft, so it takes the same guard as
                // every other context change.
                let allowed = match input.workspace.current() {
                    Some(track) if pane == LyricsPane::Cues => {
                        self.allow_context_change(editor, track)
                    }
                    _ => true,
                };
                if allowed {
                    editor.set_pane(target);
                }
            }
        }

        let body = head.body;
        let pane = editor.pane();
        let Some(track) = input.workspace.current() else {
            widgets::draw_text(
                d,
                font,
                "Open a track to edit its lyrics.",
                body.x + layout::BODY_PAD_X,
                body.y + layout::BODY_PAD_Y,
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
            editor.column.lyrics_line = format!(
                "pane={} rect={} seg={seg_report} track=none",
                pane.name(),
                rect_token(column)
            );
            return;
        };
        let sample = self
            .sample_cue_for(pane, input.workspace, input.time_seconds)
            .map_or_else(|| "none".to_owned(), |id| format!("#{id}"));
        let prefix = format!(
            "pane={} rect={} head={} seg={seg_report}",
            pane.name(),
            rect_token(column),
            rect_token(head.head)
        );
        match pane {
            LyricsPane::Cues => {
                let line = self.cues_pane(d, input, editor, track, body, commands);
                editor.column.lyrics_line = format!("{prefix} {line} sample={sample}");
            }
            LyricsPane::Style => {
                let line = self.style_pane(d, input, editor, track, body);
                editor.column.lyrics_line = format!(
                    "{prefix} scroll={:.0} sample={sample}",
                    editor.column.scroll
                );
                editor.column.style_line = line;
            }
            LyricsPane::Effects => {
                let line = self.effects_pane(d, input, editor, track, body);
                editor.column.lyrics_line = format!(
                    "{prefix} scroll={:.0} sample={sample}",
                    editor.column.scroll
                );
                editor.column.style_line = line;
            }
            LyricsPane::Fonts => {
                let area = UiRect::new(
                    body.x + layout::BODY_PAD_X,
                    body.y + layout::BODY_PAD_Y,
                    (body.width - layout::BODY_PAD_X * 2.0).max(0.0),
                    (body.height - layout::BODY_PAD_Y * 2.0).max(0.0),
                );
                self.font_browser_pane(d, input, area, commands);
                // A download that started goes back to Style, where its
                // progress lands under the face chips (§3.2).
                if self.font_browser.import_progress().is_some() {
                    editor.set_pane(LyricsPane::Style);
                }
                editor.column.lyrics_line = format!(
                    "{prefix} browser={}:{} fonts=[{}] sample={sample}",
                    rect_token(area),
                    self.font_browser.drawn_now(),
                    self.font_browser.describe()
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Cues
    // -----------------------------------------------------------------------

    /// The card, the list and the footer. Returns the report line's middle.
    #[allow(
        clippy::too_many_lines,
        reason = "one pane top to bottom; split by part it would pass the same eight values around"
    )]
    fn cues_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        body: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) -> String {
        use raylib::consts::KeyboardKey as Key;

        let font = input.fonts.ui();
        let authored = input.fonts.authored();
        let field = (authored.name(), authored.repertoire());
        editor.authored = authored_evidence(
            field,
            field,
            track.lyrics.cues().iter().map(|cue| cue.text.as_str()),
            editor.text.edit.text(),
        );
        let playhead = input.time_seconds;
        let document = &track.lyrics;
        // An empty card under a line that is playing reads as nothing to do:
        // with nothing bound and the list following, the card takes the line
        // under the playhead (bound, not seeked, and the lane's own selection
        // left alone). Once anything is bound it stays until the user moves.
        if editor.selected_id == 0 && !editor.draft_new && editor.list_follow_selection {
            if let Some(live) = document.at_time(playhead) {
                editor.select(document, live.id);
            }
        }

        // The status line: an armed tap run, else an overlap that keeps the
        // selected line off screen (the band's hint row, same precedence).
        let tap_status = editor.tap_status();
        let notice = (!editor.draft_new)
            .then(|| shadow_notice(document, editor.selected_id))
            .flatten();
        let status = match (tap_status, notice) {
            (Some(status), _) => Some((status, color::ui_ink())),
            (None, Some(warning)) => Some((warning, color::ui_warning())),
            (None, None) => None,
        };
        let legend_rows = if legend_width(font, document, false) <= body.width - 28.0 {
            1
        } else {
            2
        };
        let cues = layout::cues_layout(
            body,
            CuesSpec {
                legend_rows,
                status_line: status.is_some(),
            },
        );
        let card = cues.card;

        // --- The card ------------------------------------------------------
        widgets::rounded_box(
            d,
            card.card,
            layout::CARD_RADIUS,
            color::ui_raised(),
            color::ui_rule(),
            1.0,
        );
        let selected_index = document.index_of(editor.selected_id);
        let bound = editor.draft_new || selected_index.is_some();
        let micro_y = |rect: UiRect| (rect.y + (rect.height - metric::UI_FONT_MICRO) * 0.5).round();
        let title_x = card.title.x;
        if editor.draft_new {
            widgets::draw_micro(
                d,
                font,
                title_x,
                micro_y(card.title),
                &[
                    MicroRun::ink("New cue"),
                    MicroRun::muted(" · at the playhead"),
                ],
            );
        } else if let Some(index) = selected_index {
            let number = (index + 1).to_string();
            let total = format!(" of {}", document.len());
            let width = widgets::draw_micro(
                d,
                font,
                title_x,
                micro_y(card.title),
                &[
                    MicroRun::muted("Cue "),
                    MicroRun::ink(&number),
                    MicroRun::muted(&total),
                ],
            );
            let origin = document.cues()[index].origin;
            let rest = card.title.x + card.title.width - (title_x + width + 10.0);
            widgets::status_dot_label(
                d,
                font,
                UiRect::new(
                    title_x + width + 10.0,
                    card.title.y,
                    rest.max(0.0),
                    card.title.height,
                ),
                StatusTone::Ink,
                origin_legend_word(origin),
            );
            // The dot in the origin's own colour, over the status dot's ink:
            // the colour code is the point, and `StatusTone` has no origins.
            let dot = UiRect::new(
                title_x + width + 10.0,
                (card.title.y + (card.title.height - widgets::STATUS_DOT_SIZE) * 0.5).round(),
                widgets::STATUS_DOT_SIZE,
                widgets::STATUS_DOT_SIZE,
            );
            d.draw_circle_v(
                Vector2::new(dot.x + dot.width * 0.5, dot.y + dot.height * 0.5),
                widgets::STATUS_DOT_SIZE * 0.5,
                origin_color(origin),
            );
        } else {
            widgets::draw_micro(
                d,
                font,
                title_x,
                micro_y(card.title),
                &[MicroRun::muted("No cue selected")],
            );
        }
        for (rect, forward, index, icon, word) in [
            (
                card.previous,
                false,
                action::PREVIOUS,
                Icon::ChevronLeft,
                "<",
            ),
            (card.next, true, action::NEXT, Icon::ChevronRight, ">"),
        ] {
            let id = widgets::widget_id(ns::ACTIONS, index);
            let state = self.ghost_icon(d, input.fonts, id, rect, icon, word, false);
            self.widgets.hint(
                d,
                state,
                id,
                rect,
                if forward {
                    "Next cue, and play from it"
                } else {
                    "Previous cue, and play from it"
                },
            );
            if state.clicked && self.allow_context_change(editor, track) {
                editor.step_selection(document, forward, playhead);
            }
        }

        if bound {
            let event = editor.text.draw_with_face(
                d,
                authored,
                card.text,
                19.0,
                "Type lyric content",
                true,
            );
            if event.flattened {
                self.notify(
                    Severity::Info,
                    "Pasted as one line",
                    "A cue is a single line, so line breaks became spaces.",
                );
            }
            if let Some(error) = event.rejected {
                self.notify(
                    Severity::Warning,
                    "Nothing was pasted",
                    match error {
                        TextEditError::TooLong => {
                            "The clipboard is longer than a cue can hold, and a cue is never truncated."
                        }
                        TextEditError::Rejected => {
                            "The clipboard holds a control character a cue cannot contain."
                        }
                        TextEditError::Empty => "There was nothing on the clipboard.",
                    },
                );
            }
            if event.clipboard_unavailable {
                self.notify(
                    Severity::Warning,
                    "Nothing was pasted",
                    "The clipboard could not be read as text.",
                );
            }
        } else {
            widgets::rounded_box(
                d,
                card.text,
                4.0,
                color::ui_lane_trough(),
                color::ui_rule(),
                1.0,
            );
            widgets::draw_text(
                d,
                font,
                "Pick a line below, or add one at the playhead.",
                card.text.x + 10.0,
                (card.text.y + (card.text.height - metric::UI_FONT_LABEL) * 0.5).round(),
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
        }

        let duration = document.duration_seconds();
        for (row, label, is_start, id_base) in [
            (card.start, "Start", true, form_id::START_TIME),
            (card.end, "End", false, form_id::END_TIME),
        ] {
            self.column_time_row(
                d, input, editor, row, label, is_start, bound, playhead, duration, id_base,
                cues.wide,
            );
        }
        widgets::fill(d, card.rule, color::ui_rule());

        // The timing ladder: Tap, Split, Merge, undo, redo — every control
        // and key the band has, in one row.
        for (control, rect) in [
            (TimingControl::Tap, card.tap),
            (TimingControl::Split, card.split),
            (TimingControl::Merge, card.merge),
            (TimingControl::Undo, card.undo),
            (TimingControl::Redo, card.redo),
        ] {
            let (enabled, active) = match control {
                TimingControl::Tap => (!document.is_empty(), editor.tap_is_armed()),
                TimingControl::Split | TimingControl::Merge => {
                    (editor.selected_id != 0 && !editor.draft_new, false)
                }
                TimingControl::Undo => (editor.can_undo(), false),
                TimingControl::Redo => (editor.can_redo(), false),
            };
            let id = widgets::widget_id(ns::ACTIONS, control.widget_index());
            let icon = match control {
                TimingControl::Undo => Some(Icon::Undo),
                TimingControl::Redo => Some(Icon::Redo),
                _ => None,
            };
            if !enabled {
                match icon {
                    Some(icon) => self.disabled_icon(d, input.fonts, rect, icon, control.label()),
                    None => self.widgets.disabled_button(
                        d,
                        font,
                        rect,
                        control.label(),
                        Some(metric::UI_FONT_CAPTION),
                    ),
                }
                continue;
            }
            let state = match icon {
                Some(icon) => {
                    self.ghost_icon(d, input.fonts, id, rect, icon, control.label(), false)
                }
                None => self.widgets.text_button(
                    d,
                    font,
                    id,
                    rect,
                    control.label(),
                    active,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_CAPTION),
                ),
            };
            self.widgets.hint(
                d,
                state,
                id,
                rect,
                &format!("{} ({})", control.label(), control.key()),
            );
            if !state.clicked {
                continue;
            }
            match control {
                TimingControl::Tap => self.toggle_tap_run(editor, track, playhead),
                TimingControl::Split => self.split_selected(editor, track, playhead),
                TimingControl::Merge => self.merge_selected(editor, track),
                TimingControl::Undo => editor.request_history(HistoryStep::Undo),
                TimingControl::Redo => editor.request_history(HistoryStep::Redo),
            }
        }

        // Apply (strong), Discard, Delete (danger text).
        // Apply is offered only for a change: an ink slab on an untouched cue
        // would say a commit is waiting when none is (the mockup dims it).
        let dirty = editor.has_unsaved_draft(document);
        let apply_enabled = match editor.draft_edit() {
            Some(edit) if bound && dirty => {
                if self
                    .widgets
                    .text_button(
                        d,
                        font,
                        widgets::widget_id(ns::FORM, form_id::APPLY),
                        card.apply,
                        "Apply",
                        false,
                        ButtonStyle::Strong,
                        Some(metric::UI_FONT_CAPTION),
                    )
                    .clicked
                {
                    apply_draft(editor, track, edit);
                }
                true
            }
            _ => {
                self.widgets.disabled_button(
                    d,
                    font,
                    card.apply,
                    "Apply",
                    Some(metric::UI_FONT_CAPTION),
                );
                false
            }
        };
        let discard_label = if editor.draft_new {
            "Cancel"
        } else {
            "Discard"
        };
        if bound
            && self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::FORM, form_id::DISCARD),
                    card.discard,
                    discard_label,
                    false,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
        {
            editor.clear_draft();
        }
        let deletable = !editor.draft_new && selected_index.is_some();
        if deletable {
            let id = widgets::widget_id(ns::FORM, form_id::DELETE);
            let state = self.widgets.button(d, id, card.delete);
            if state.hovered {
                widgets::control_box(d, card.delete, color::ui_hover(), color::ui_hover(), 1.0);
            }
            widgets::text_label(
                d,
                font,
                card.delete,
                "Delete",
                metric::UI_FONT_CAPTION,
                color::ui_danger(),
            );
            if state.clicked {
                editor.push(LyricsEdit::Delete {
                    id: editor.selected_id,
                });
                editor.clear_draft();
            }
        }
        if !card.hint.is_empty() {
            let hint = [MicroRun::muted("Ctrl+Enter taps the next line")];
            let width = widgets::micro_width(font, &hint);
            if width <= card.hint.width {
                widgets::draw_micro(
                    d,
                    font,
                    card.hint.x + card.hint.width - width,
                    micro_y(card.hint),
                    &hint,
                );
            }
        }
        if let Some((line, tint)) = &status {
            let (fitted, _) = musializer_core::ui::row_typography::truncate_label(
                line,
                card.status.width,
                Some(|text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION - 1.0)),
                musializer_core::ui::row_typography::UI_ROW_LABEL_CAPACITY,
            );
            widgets::draw_text(
                d,
                font,
                &fitted,
                card.status.x,
                card.status.y + 1.0,
                metric::UI_FONT_CAPTION - 1.0,
                *tint,
            );
        }
        // Ctrl+Enter, the oracle's shortcut (`:1420-1423`), as the band has it.
        if editor.text.is_focused()
            && (d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL))
            && d.is_key_pressed(Key::KEY_ENTER)
        {
            if let Some(edit) = editor.draft_edit() {
                apply_draft(editor, track, edit);
            }
        }

        // --- The list ------------------------------------------------------
        let list = cues.list;
        widgets::draw_micro(
            d,
            font,
            list.head.x,
            micro_y(list.head),
            &[MicroRun::muted("All cues")],
        );
        let follow_id = widgets::widget_id(ns::ACTIONS, action::FOLLOW);
        let follow_state = self.widgets.button(d, follow_id, list.follow);
        let follow_on = editor.list_follow_selection;
        let (chip_fill, chip_edge, chip_ink) = if follow_on {
            (
                color::ui_selected(),
                color::ui_selected_edge(),
                color::ui_ink(),
            )
        } else if follow_state.hovered {
            (color::ui_hover(), color::ui_control_edge(), color::ui_ink())
        } else {
            (
                color::ui_surface(),
                color::ui_control_edge(),
                color::ui_muted(),
            )
        };
        widgets::rounded_box(
            d,
            list.follow,
            list.follow.height * 0.5,
            chip_fill,
            chip_edge,
            1.0,
        );
        let ring = Vector2::new(
            list.follow.x + 14.0,
            list.follow.y + list.follow.height * 0.5,
        );
        d.draw_ring(ring, 3.0, 4.5, 0.0, 360.0, 16, chip_ink);
        if follow_on {
            d.draw_circle_v(ring, 1.5, chip_ink);
        }
        widgets::draw_text(
            d,
            font,
            "follows the playhead",
            list.follow.x + 24.0,
            (list.follow.y + (list.follow.height - 12.0) * 0.5).round(),
            12.0,
            chip_ink,
        );
        self.widgets.hint(
            d,
            follow_state,
            follow_id,
            list.follow,
            "Keep the line being sung in view. Scrolling the list turns this off.",
        );
        if follow_state.clicked {
            editor.list_follow_selection = !follow_on;
        }

        let cue_list = document.cues();
        let live = document
            .at_time(playhead)
            .and_then(|cue| document.index_of(cue.id));
        // While playing, the list follows the song; paused, the line being
        // edited. Either way, the line under the playhead when nothing is
        // bound — a lyric sheet scrolling with the song (§3.2).
        let last_started = cue_list
            .iter()
            .rposition(|cue| cue.start_seconds <= playhead);
        let focus = if input.playing {
            live.or(last_started)
        } else {
            selected_index.or(live).or(last_started)
        }
        .unwrap_or(0);
        let mouse = input.ui_scale.mouse(d);
        if list.rows.contains_point(mouse.x, mouse.y) {
            let wheel = d.get_mouse_wheel_move();
            if wheel != 0.0 {
                editor.list_follow_selection = false;
                editor.list_first =
                    (editor.list_first as i64 - (wheel * 3.0) as i64).max(0) as usize;
            }
        }
        let first = layout::list_first(
            focus,
            list.visible,
            cue_list.len(),
            editor.list_follow_selection,
            editor.list_first,
        );
        editor.list_first = first;
        if cue_list.is_empty() {
            widgets::draw_text(
                d,
                font,
                "No lyric cues yet. Add one at the playhead.",
                list.rows.x,
                list.rows.y + 8.0,
                metric::UI_FONT_LABEL,
                color::ui_muted(),
            );
        }
        let mut rows_report = String::new();
        let mut current_report = "none".to_owned();
        let row_size = metric::UI_FONT_AUTHORED_ROW;
        for slot in 0..list.visible {
            let index = first + slot;
            let Some(cue) = cue_list.get(index) else {
                break;
            };
            let row = layout::row_rect(list.rows, slot);
            let parts = layout::row_parts(row);
            let id = widgets::widget_id(ns::ROWS, cue.id as u32);
            let state = self.widgets.button(d, id, row);
            let selected = cue.id == editor.selected_id && !editor.draft_new;
            let current = live == Some(index);
            let sung = cue.end_seconds <= playhead;
            if selected {
                widgets::rounded_box(
                    d,
                    row,
                    4.0,
                    color::ui_selected(),
                    color::ui_selected_edge(),
                    1.0,
                );
            } else if state.hovered {
                widgets::rounded_box(d, row, 4.0, color::ui_hover(), color::ui_hover(), 1.0);
            }
            if current {
                // An exact opaque 2 px column in the accent: the current line
                // is *now* (5a), and the gate reads this tick pixel for pixel.
                widgets::fill(d, parts.tick, color::accent());
                current_report = format!(
                    "#{} time@{} tick={}",
                    cue.id,
                    rect_token(parts.time),
                    rect_token(parts.tick)
                );
            }
            d.draw_circle_v(
                Vector2::new(
                    parts.dot.x + parts.dot.width * 0.5,
                    parts.dot.y + parts.dot.height * 0.5,
                ),
                layout::ROW_DOT * 0.5,
                origin_color(cue.origin),
            );
            let time = widgets::format_timestamp(cue.start_seconds);
            widgets::draw_text_tabular(
                d,
                font,
                &time,
                parts.time.x,
                (parts.time.y + (parts.time.height - metric::UI_FONT_CAPTION) * 0.5).round(),
                metric::UI_FONT_CAPTION,
                if current {
                    color::accent()
                } else {
                    color::ui_muted()
                },
            );
            if parts.text.width > 8.0 {
                let fitted = authored.ellipsize(&cue.text, parts.text.width, row_size, 0.0);
                let mut clip = widgets::begin_scissor(d, parts.text, input.ui_scale);
                authored.draw_text(
                    &mut clip,
                    &fitted,
                    Vector2::new(
                        parts.text.x,
                        (parts.text.y + (parts.text.height - row_size) * 0.5).round() - 1.0,
                    ),
                    row_size,
                    0.0,
                    if sung && !current {
                        color::ui_muted()
                    } else {
                        color::ui_ink()
                    },
                );
                drop(clip);
            }
            if !rows_report.is_empty() {
                rows_report.push(';');
            }
            rows_report.push_str(&format!("#{}:{}", cue.id, rect_token(row)));
            if state.clicked && (selected || self.allow_context_change(editor, track)) {
                editor.select_and_seek(document, cue.id);
            }
        }
        if cue_list.len() > list.visible && list.visible > 0 {
            let travel_height = list.rows.height;
            let thumb = 24.0f32.max(travel_height * list.visible as f32 / cue_list.len() as f32);
            let amount = first as f32 / (cue_list.len() - list.visible) as f32;
            widgets::fill(
                d,
                UiRect::new(
                    list.rows.x + list.rows.width + 4.0,
                    list.rows.y + (travel_height - thumb) * amount,
                    2.0,
                    thumb,
                ),
                color::ui_rule(),
            );
        }

        // --- The footer ----------------------------------------------------
        let footer = cues.footer;
        widgets::fill(d, footer.rule, color::ui_rule());
        draw_legend(d, font, document, footer.legend, legend_rows);
        let add_label = format!(
            "{} {}",
            if cues.wide { "Add a cue at" } else { "Add at" },
            widgets::format_timestamp(playhead)
        );
        let add_id = widgets::widget_id(ns::ACTIONS, 1);
        let add_state = self.icon_word_button(
            d,
            input.fonts,
            add_id,
            footer.add,
            Icon::Plus,
            &add_label,
            ButtonStyle::Neutral,
        );
        if add_state.clicked && self.allow_context_change(editor, track) {
            editor.begin_new(document, playhead);
        }
        let exportable = !document.is_empty() && document.duration_seconds() > 0.0;
        let import_id = widgets::widget_id(ns::ACTIONS, 8);
        let import_state = self.widgets.text_button(
            d,
            font,
            import_id,
            footer.import,
            "Import\u{2026}",
            false,
            ButtonStyle::Ghost,
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(
            d,
            import_state,
            import_id,
            footer.import,
            "Replace every cue from a .lyrics.tsv. Ctrl+Z puts the old ones back.",
        );
        if import_state.clicked && self.allow_context_change(editor, track) {
            commands.push(ShellCommand::ImportLyrics);
        }
        if exportable {
            let export_id = widgets::widget_id(ns::ACTIONS, 7);
            let state = self.widgets.text_button(
                d,
                font,
                export_id,
                footer.export,
                "Export\u{2026}",
                false,
                ButtonStyle::Ghost,
                Some(metric::UI_FONT_CAPTION),
            );
            self.widgets.hint(
                d,
                state,
                export_id,
                footer.export,
                "Write these cues to a .lyrics.tsv. The project is not changed.",
            );
            if state.clicked {
                commands.push(ShellCommand::ExportLyrics);
            }
        } else {
            widgets::text_label(
                d,
                font,
                footer.export,
                "Export\u{2026}",
                metric::UI_FONT_CAPTION,
                color::ui_disabled(),
            );
        }

        let last = (first + list.visible).min(cue_list.len());
        format!(
            "card={} cue={} prev@{} next@{} text@{} start@{} end@{} apply@{}:{} \
             list@{} rows={}..{}/{} row@={} current={} selected={} follow={} follow@{} \
             footer@{} footer=add@{},import@{},export@{}",
            rect_token(card.card),
            if editor.draft_new {
                "new".to_owned()
            } else {
                selected_index
                    .map_or_else(|| "none".to_owned(), |_| format!("#{}", editor.selected_id))
            },
            rect_token(card.previous),
            rect_token(card.next),
            rect_token(card.text),
            rect_token(card.start.field),
            rect_token(card.end.field),
            rect_token(card.apply),
            if apply_enabled { "enabled" } else { "disabled" },
            rect_token(list.rows),
            if cue_list.is_empty() { 0 } else { first + 1 },
            last,
            cue_list.len(),
            if rows_report.is_empty() {
                "none".to_owned()
            } else {
                rows_report
            },
            current_report,
            if editor.selected_id == 0 || editor.draft_new {
                "none".to_owned()
            } else {
                format!("#{}", editor.selected_id)
            },
            u8::from(editor.list_follow_selection),
            rect_token(list.follow),
            rect_token(footer.footer),
            rect_token(footer.add),
            rect_token(footer.import),
            rect_token(footer.export),
        )
    }

    /// One START or END row on the card: label, the click-to-type time, − +,
    /// and set-to-the-playhead. Same ids, steps, hold-repeat and clamping as
    /// the band's `time_row`; only the geometry is the column's.
    #[allow(
        clippy::too_many_arguments,
        reason = "the band's `time_row` arguments plus whether the card is bound and how wide it is"
    )]
    fn column_time_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        row: layout::TimeRow,
        label: &str,
        is_start: bool,
        bound: bool,
        playhead: f64,
        duration: f64,
        id_base: u32,
        wide: bool,
    ) {
        use raylib::consts::KeyboardKey as Key;

        let font = input.fonts.ui();
        widgets::draw_micro(
            d,
            font,
            row.label.x,
            (row.label.y + (row.label.height - metric::UI_FONT_MICRO) * 0.5).round(),
            &[MicroRun::muted(label)],
        );
        if !bound {
            widgets::rounded_box(
                d,
                row.field,
                4.0,
                color::ui_lane_trough(),
                color::ui_rule(),
                1.0,
            );
            widgets::draw_text(
                d,
                font,
                "--:--.---",
                row.field.x + 10.0,
                (row.field.y + (row.field.height - metric::UI_FONT_LABEL) * 0.5).round(),
                metric::UI_FONT_LABEL,
                color::ui_disabled(),
            );
            return;
        }
        let (mut value, other) = if is_start {
            (editor.draft_start, editor.draft_end)
        } else {
            (editor.draft_end, editor.draft_start)
        };
        let fine = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
        let coarse = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
        let step = lyric_lane_edit::cue_nudge_step_seconds(fine, coarse);

        widgets::rounded_box(
            d,
            row.field,
            4.0,
            color::ui_lane_trough(),
            color::ui_control_edge(),
            1.0,
        );
        let text_y = ((row.field.height - metric::UI_FONT_LABEL) * 0.5).round();
        if let Some(typed) = self.time_readout(
            d,
            font,
            input.fonts.authored(),
            editor,
            row.field,
            is_start,
            value,
            id_base,
            (metric::UI_FONT_LABEL, text_y, 10.0, metric::UI_FONT_LABEL),
        ) {
            value = typed;
        }
        let hint = format!("{step} s. Ctrl for 0.01 s, Shift for 1 s. Hold to repeat.");
        for (index, rect, glyph) in [(0u32, row.minus, "-"), (1, row.plus, "+")] {
            let id = widgets::widget_id(ns::FORM, id_base + index);
            let state = self.widgets.text_button(
                d,
                font,
                id,
                rect,
                glyph,
                false,
                ButtonStyle::Ghost,
                Some(metric::UI_FONT_LABEL),
            );
            self.widgets.hint(d, state, id, rect, &hint);
            let repeating = editor.nudge_repeat.tick(id, state.pressed);
            if state.clicked || repeating {
                value += if index == 0 { -step } else { step };
            }
        }
        let set_id = widgets::widget_id(ns::FORM, id_base + 2);
        let icon_rect = UiRect::new(
            row.playhead.x,
            row.playhead.y,
            layout::width::STEP,
            row.playhead.height,
        );
        let mut set = self.ghost_icon(
            d,
            input.fonts,
            set_id,
            icon_rect,
            Icon::Crosshairs,
            "Set",
            false,
        );
        self.widgets
            .hint(d, set, set_id, icon_rect, "Move this end to the playhead");
        if wide {
            let word = UiRect::new(
                icon_rect.x + icon_rect.width,
                row.playhead.y,
                (row.playhead.width - icon_rect.width).max(0.0),
                row.playhead.height,
            );
            let word_id =
                widgets::widget_id(ns::ACTIONS, action::PLAYHEAD_WORD + u32::from(!is_start));
            let state = self.widgets.button(d, word_id, word);
            if state.hovered {
                widgets::control_box(d, word, color::ui_hover(), color::ui_hover(), 1.0);
            }
            widgets::draw_text(
                d,
                font,
                "playhead",
                word.x + 2.0,
                (word.y + (word.height - metric::UI_FONT_CAPTION) * 0.5).round(),
                metric::UI_FONT_CAPTION,
                if state.hovered {
                    color::ui_ink()
                } else {
                    color::ui_muted()
                },
            );
            set.clicked |= state.clicked;
        }
        if set.clicked {
            value = playhead;
        }
        if is_start {
            editor.draft_start = lyric_lane_edit::clamp_form_start(value, other);
        } else {
            editor.draft_end = lyric_lane_edit::clamp_form_end(value, other, duration);
        }
    }

    // -----------------------------------------------------------------------
    // Style
    // -----------------------------------------------------------------------

    /// TYPE, COLOUR, BACKING, PLACE and the note, scrolling in the body.
    /// Returns the `caption style:` line.
    #[allow(
        clippy::too_many_lines,
        reason = "one pane top to bottom, as the band's caption_style_form is"
    )]
    fn style_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        body: UiRect,
    ) -> String {
        let font = input.fonts.ui();
        let mut style = track.caption_style.clone();
        let defaults = CaptionStyle::default();
        let mut changed = false;
        let imported = style.font.as_ref().map(|asset| asset.family.clone());
        let progress = self.font_browser.import_progress();
        let tuners = match style.box_style {
            CaptionBox::Shadow => 2,
            CaptionBox::Plate => 1,
            CaptionBox::None => 0,
        };
        let spec = StyleSpec {
            faces: if imported.is_some() { 3 } else { 2 },
            progress: progress.is_some(),
            tuners,
            picker: match editor.picker.target {
                Some(CaptionColor::Ink) => Some(PickerAt::Ink),
                Some(CaptionColor::Plate) => Some(PickerAt::Plate),
                _ => None,
            },
        };
        let pane = layout::style_layout(body, spec);
        let offset = self.scroll_body(d, input, editor, body, pane.content_height);
        let s = |rect: UiRect| layout::scrolled(rect, offset);
        let old_clip = self.widgets.set_input_clip(Some(body));
        let mut clip = widgets::begin_scissor(d, body, input.ui_scale);
        let d = &mut *clip;

        // TYPE
        widgets::section_head(d, font, s(pane.type_head), &[MicroRun::muted("Type")]);
        let mut faces_report = Vec::new();
        let face_names: Vec<(CaptionFace, String)> = {
            let mut names = vec![
                (CaptionFace::Alegreya, "Alegreya".to_owned()),
                (CaptionFace::SpaceGrotesk, "Grotesk".to_owned()),
            ];
            if let Some(family) = &imported {
                names.push((CaptionFace::Imported, family.clone()));
            }
            names
        };
        for (index, (face, name)) in face_names.iter().enumerate() {
            let chip = s(pane.face_chips[index]);
            let id = widgets::widget_id(ns::CAPTION, caption_id::FACE + index as u32);
            let on = style.face == *face;
            let state = self.widgets.button(d, id, chip);
            let look = widgets::button_look(ButtonStyle::Neutral, on, state.hovered);
            widgets::control_box(d, chip, look.fill, look.edge, 1.0);
            {
                // One caption face on loan at a time (AGENTS.md trap): this
                // borrow ends before the next chip asks for its own.
                let sample = input.fonts.caption_at(*face, 20.0, "Aa");
                d.draw_text_ex(
                    sample.face(),
                    "Aa",
                    Vector2::new(chip.x + 10.0, chip.y + 4.0),
                    20.0,
                    0.0,
                    color::ui_ink(),
                );
            }
            let (word, _) = musializer_core::ui::row_typography::truncate_label(
                &name.to_uppercase(),
                chip.width - 20.0,
                Some(|text: &str| widgets::micro_width(font, &[MicroRun::muted(text)])),
                musializer_core::ui::row_typography::UI_ROW_LABEL_CAPACITY,
            );
            widgets::draw_micro(
                d,
                font,
                chip.x + 10.0,
                chip.y + 27.0,
                &[MicroRun::muted(&word)],
            );
            if state.clicked && !on {
                style.face = *face;
                changed = true;
            }
            faces_report.push(format!(
                "{}{}@{}",
                match face {
                    CaptionFace::Alegreya => "alegreya",
                    CaptionFace::SpaceGrotesk => "grotesk",
                    CaptionFace::Imported => "imported",
                },
                if on { "*" } else { "" },
                rect_token(chip)
            ));
            if *face == CaptionFace::Imported {
                // Removing the asset drops the face that named it in the same
                // step, or the project validates as a face with nothing behind it.
                let close = UiRect::new(chip.x + chip.width - 22.0, chip.y + 2.0, 20.0, 20.0);
                let remove_id = widgets::widget_id(ns::CAPTION, caption_id::REMOVE_FACE);
                let state =
                    self.ghost_icon(d, input.fonts, remove_id, close, Icon::Times, "x", false);
                self.widgets.hint(
                    d,
                    state,
                    remove_id,
                    close,
                    "Remove the imported face from the project",
                );
                if state.clicked {
                    style.face = CaptionFace::Alegreya;
                    style.font = None;
                    changed = true;
                    self.font_browser.clear_selection();
                }
            }
        }
        let import_chip = s(*pane.face_chips.last().unwrap_or(&UiRect::default()));
        let import_id = widgets::widget_id(ns::CAPTION, caption_id::IMPORT_FACE);
        let state = self.widgets.button(d, import_id, import_chip);
        if state.hovered {
            widgets::control_box(d, import_chip, color::ui_hover(), color::ui_hover(), 1.0);
        }
        dashed_outline(d, import_chip, color::ui_control_edge());
        self.icon_glyph(
            d,
            input.fonts,
            UiRect::new(import_chip.x, import_chip.y + 6.0, import_chip.width, 16.0),
            Icon::Plus,
            "+",
            color::ui_muted(),
        );
        let import_word = [MicroRun::muted("Import")];
        let word_width = widgets::micro_width(font, &import_word);
        widgets::draw_micro(
            d,
            font,
            (import_chip.x + (import_chip.width - word_width) * 0.5).round(),
            import_chip.y + 27.0,
            &import_word,
        );
        self.widgets.hint(
            d,
            state,
            import_id,
            import_chip,
            "Find a face on Google Fonts and bundle it into the project",
        );
        if state.clicked {
            editor.set_pane(LyricsPane::Fonts);
        }
        faces_report.push(format!("import@{}", rect_token(import_chip)));
        let mut progress_report = "none".to_owned();
        if let (Some(block), Some(progress)) = (pane.progress, progress.as_ref()) {
            let block = layout::ProgressLayout {
                block: s(block.block),
                family: s(block.family),
                cancel: s(block.cancel),
                sweep: s(block.sweep),
                status: s(block.status),
            };
            let family = format!("Importing · {}", progress.family);
            widgets::draw_micro(
                d,
                font,
                block.family.x,
                (block.family.y + (block.family.height - metric::UI_FONT_MICRO) * 0.5).round(),
                &[MicroRun::ink(&family)],
            );
            if progress.cancelling {
                widgets::text_label(
                    d,
                    font,
                    block.cancel,
                    "Stopping",
                    metric::UI_FONT_CAPTION,
                    color::ui_disabled(),
                );
            } else if self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(ns::CAPTION, column_id::IMPORT_CANCEL),
                    block.cancel,
                    "Cancel",
                    false,
                    ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked
            {
                self.font_browser.cancel_import();
            }
            // Indeterminate: the importer reports no percentage, so none is
            // drawn — a warm-grey segment travels the track instead (§3.2).
            widgets::rounded_box(
                d,
                block.sweep,
                2.0,
                color::ui_selected(),
                color::ui_selected(),
                1.0,
            );
            let phase = (d.get_time() * 0.6).fract() as f32;
            let segment = block.sweep.width * 0.3;
            let start = block.sweep.x - segment + (block.sweep.width + segment) * phase;
            let left = start.max(block.sweep.x);
            let right = (start + segment).min(block.sweep.x + block.sweep.width);
            if right > left {
                widgets::fill(
                    d,
                    UiRect::new(left, block.sweep.y, right - left, block.sweep.height),
                    color::ui_muted(),
                );
            }
            widgets::draw_text(
                d,
                font,
                &progress.status,
                block.status.x,
                block.status.y,
                12.0,
                color::ui_muted(),
            );
            progress_report = format!(
                "{}@{} cancel@{}",
                progress.family.replace(' ', "_"),
                rect_token(block.block),
                rect_token(block.cancel)
            );
        }

        let mut sliders_report = Vec::new();
        #[rustfmt::skip]
        let rows: [(LabelledRow, &str, f64, f64, u32, u8, &str); 2] = [
            (pane.size,  "Size",  caption::SIZE_MINIMUM,  caption::SIZE_MAXIMUM,  column_id::SIZE,  0,
             "Caption height as a fraction of the frame, so exports match the preview"),
            (pane.width, "Width", caption::WIDTH_MINIMUM, caption::WIDTH_MAXIMUM, column_id::WIDTH, 1,
             "How wide a caption may grow before it wraps"),
        ];
        for (row, label, minimum, maximum, id, which, tip) in rows {
            let (value, default) = match which {
                0 => (&mut style.size_scale, defaults.size_scale),
                _ => (&mut style.width_scale, defaults.width_scale),
            };
            let precision = usize::from(which == 0);
            if let Some(rect) = self.column_slider(
                d,
                font,
                s(row.row),
                label,
                value,
                default,
                (minimum, maximum),
                id,
                precision,
                layout::PANE_LABEL,
                tip,
            ) {
                changed = true;
                let _ = rect;
            }
            sliders_report.push(format!(
                "{}@{}",
                label.to_lowercase(),
                rect_token(s(row.row))
            ));
        }

        // COLOUR
        widgets::section_head(d, font, s(pane.colour_head), &[MicroRun::muted("Colour")]);
        let mut ink_report = String::new();
        let mut plate_report = String::new();
        for (row, label, target, presets, plus_id, swatch_base, picker) in [
            (
                pane.ink,
                "Ink",
                CaptionColor::Ink,
                &CAPTION_TEXT_SWATCHES[..],
                column_id::INK_PLUS,
                caption_id::INK_SWATCHES,
                pane.ink_picker,
            ),
            (
                pane.plate,
                "Plate",
                CaptionColor::Plate,
                &CAPTION_BOX_SWATCHES[..],
                column_id::PLATE_PLUS,
                caption_id::PLATE_SWATCHES,
                pane.plate_picker,
            ),
        ] {
            let row = LabelledRow {
                row: s(row.row),
                label: s(row.label),
                control: s(row.control),
            };
            pane_label(d, font, row.label, label);
            let current = target.read(&style);
            let custom = !presets.contains(&current);
            let mut cells: Vec<u32> = presets.to_vec();
            if custom {
                cells.push(current);
            }
            let rects = layout::swatch_rects(row.control, cells.len() + 1);
            let report = if target == CaptionColor::Ink {
                &mut ink_report
            } else {
                &mut plate_report
            };
            for (index, rect) in rects.iter().enumerate() {
                if index == cells.len() {
                    // `+`: the free picker for this colour.
                    let id = widgets::widget_id(ns::CAPTION, plus_id);
                    let state = self.widgets.button(d, id, *rect);
                    let open = editor.picker.target == Some(target);
                    let centre =
                        Vector2::new(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
                    if open || state.hovered {
                        d.draw_circle_v(centre, layout::SWATCH * 0.5, color::ui_hover());
                    }
                    dashed_circle(d, centre, layout::SWATCH * 0.5, color::ui_control_edge());
                    self.icon_glyph(d, input.fonts, *rect, Icon::Plus, "+", color::ui_muted());
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        *rect,
                        "Any colour, with transparency: the free picker",
                    );
                    if state.clicked {
                        editor.picker.toggle(target);
                        editor.tune_target = None;
                    }
                    report.push_str(&format!("+@{}", rect_token(*rect)));
                    continue;
                }
                let packed = cells[index];
                let id = widgets::widget_id(ns::CAPTION, swatch_base + index as u32);
                let result =
                    self.widgets
                        .swatch(d, id, *rect, Color::get_color(packed), packed == current);
                if result.state.clicked {
                    if index < presets.len() {
                        target.write(&mut style, packed);
                        changed = true;
                    } else {
                        editor.picker.toggle(target);
                    }
                }
                report.push_str(&format!(
                    "{packed:08x}{}@{};",
                    if packed == current { "*" } else { "" },
                    rect_token(result.rect)
                ));
            }
            if let Some(block) = picker {
                if self.inline_picker(
                    d,
                    input,
                    editor,
                    s_picker(block, offset),
                    target,
                    &mut style,
                ) {
                    changed = true;
                }
            }
        }

        // BACKING
        widgets::section_head(d, font, s(pane.backing_head), &[MicroRun::muted("Backing")]);
        let box_index = index_of(CaptionBox::ALL, style.box_style);
        let backing = self.widgets.segmented(
            d,
            font,
            ns::CAPTION,
            caption_id::BACKING,
            s(pane.backing),
            &["None", "Shadow", "Plate"],
            Some(box_index),
            SegmentedFit::Fill,
        );
        if let Some(chosen) = backing.clicked {
            if let Some(value) = CaptionBox::ALL.get(chosen) {
                if *value != style.box_style {
                    style.box_style = *value;
                    changed = true;
                }
            }
        }
        let backing_report = backing
            .layout
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                format!(
                    "{}{}@{}",
                    ["none", "shadow", "plate"][index],
                    if index == box_index { "*" } else { "" },
                    rect_token(*item)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        #[rustfmt::skip]
        let tuner_rows: Vec<(&str, f64, f64, u32, u8, &str)> = match style.box_style {
            CaptionBox::Shadow => vec![
                ("Blur",  caption_fx::SHADOW_BLUR_MINIMUM,    caption_fx::SHADOW_BLUR_MAXIMUM,    column_id::BLUR,  0,
                 "Softens the shadow; 0 is the hard offset"),
                ("Shade", caption_fx::SHADOW_OPACITY_MINIMUM, caption_fx::SHADOW_OPACITY_MAXIMUM, column_id::SHADE, 1,
                 "How dark the shadow draws"),
            ],
            CaptionBox::Plate => vec![
                ("Round", caption_fx::PLATE_ROUNDNESS_MINIMUM, caption_fx::PLATE_ROUNDNESS_MAXIMUM, column_id::ROUND, 2,
                 "Corner roundness of the plate and its outline"),
            ],
            CaptionBox::None => Vec::new(),
        };
        for (row, (label, minimum, maximum, id, which, tip)) in pane.tuners.iter().zip(tuner_rows) {
            let (value, default) = match which {
                0 => (&mut style.effects.shadow_blur, defaults.effects.shadow_blur),
                1 => (
                    &mut style.effects.shadow_opacity,
                    defaults.effects.shadow_opacity,
                ),
                _ => (
                    &mut style.effects.plate_roundness,
                    defaults.effects.plate_roundness,
                ),
            };
            if self
                .column_slider(
                    d,
                    font,
                    s(row.row),
                    label,
                    value,
                    default,
                    (minimum, maximum),
                    id,
                    0,
                    layout::PANE_LABEL,
                    tip,
                )
                .is_some()
            {
                changed = true;
            }
            sliders_report.push(format!(
                "{}@{}",
                label.to_lowercase(),
                rect_token(s(row.row))
            ));
        }

        // PLACE
        widgets::section_head(d, font, s(pane.place_head), &[MicroRun::muted("Place")]);
        let frame = s(pane.place_frame);
        widgets::rounded_box(
            d,
            frame,
            4.0,
            color::ui_lane_trough(),
            color::ui_control_edge(),
            1.0,
        );
        let anchor_index = index_of(CaptionAnchor::ALL, style.anchor);
        let mut anchors_report = Vec::new();
        let mut place_word = "bottom centre";
        for cell in 0..9 {
            let press = s(pane.anchors[cell]);
            let dot = s(pane.anchor_dots[cell]);
            let value = anchor_value(cell);
            let id = widgets::widget_id(ns::CAPTION, caption_id::ANCHOR + cell as u32);
            let state = self.widgets.button(d, id, press);
            let centre = Vector2::new(dot.x + dot.width * 0.5, dot.y + dot.height * 0.5);
            let on = anchor_index == value;
            if on {
                place_word = ANCHOR_WORDS[cell];
                d.draw_circle_v(centre, layout::PLACE_DOT * 0.5 + 3.0, color::ui_ink());
                d.draw_circle_v(
                    centre,
                    layout::PLACE_DOT * 0.5 + 2.0,
                    color::ui_lane_trough(),
                );
                d.draw_circle_v(centre, layout::PLACE_DOT * 0.5, color::ui_ink());
            } else {
                d.draw_circle_v(
                    centre,
                    layout::PLACE_DOT * 0.5,
                    if state.hovered {
                        color::ui_muted()
                    } else {
                        color::ui_control_edge()
                    },
                );
            }
            if state.clicked && !on {
                if let Some(anchor) = CaptionAnchor::ALL.get(value) {
                    style.anchor = *anchor;
                    changed = true;
                }
            }
            anchors_report.push(format!(
                "{}{}@{}",
                ANCHOR_WORDS[cell].replace(' ', "-"),
                if on { "*" } else { "" },
                rect_token(press)
            ));
        }
        let inset = pane.inset;
        if self
            .column_slider(
                d,
                font,
                s(inset.row),
                "Inset",
                &mut style.margin_scale,
                defaults.margin_scale,
                (caption::MARGIN_MINIMUM, caption::MARGIN_MAXIMUM),
                column_id::INSET,
                0,
                inset.label.width,
                "Margin between the caption and the frame edge",
            )
            .is_some()
        {
            changed = true;
        }
        sliders_report.push(format!("inset@{}", rect_token(s(inset.row))));
        let word = s(pane.place_word);
        widgets::draw_micro(
            d,
            font,
            word.x,
            word.y + 2.0,
            &[MicroRun::muted(place_word)],
        );

        let note = s(pane.note);
        draw_note(
            d,
            font,
            note,
            "Sizes are fractions of the frame, so exports match the preview.",
        );
        drop(clip);
        self.widgets.set_input_clip(old_clip);

        if changed {
            editor.push(LyricsEdit::SetCaptionStyle(Box::new(style)));
        }
        format!(
            "pane=style groups=type@{},colour@{},backing@{},place@{} faces={} sliders={} \
             ink={} plate={} backing={} place={} anchors={} picker={} progress={} scroll={:.0}/{:.0}",
            rect_token(s(pane.type_head)),
            rect_token(s(pane.colour_head)),
            rect_token(s(pane.backing_head)),
            rect_token(s(pane.place_head)),
            faces_report.join(","),
            sliders_report.join(","),
            ink_report.trim_end_matches(';'),
            plate_report.trim_end_matches(';'),
            backing_report,
            place_word.replace(' ', "-"),
            anchors_report.join(";"),
            editor.picker.target.map_or("none", |target| target.token()),
            progress_report,
            offset,
            layout::max_scroll(pane.content_height, body),
        )
    }

    // -----------------------------------------------------------------------
    // Effects
    // -----------------------------------------------------------------------

    /// GLOW, PULSE and HUE, each drive with a live meter per source, `~ Tune`
    /// opening the mapping editor under its row. Returns the `caption style:`
    /// line.
    #[allow(clippy::too_many_lines, reason = "one pane top to bottom")]
    fn effects_pane(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        track: &Track,
        body: UiRect,
    ) -> String {
        let font = input.fonts.ui();
        let mut style = track.caption_style.clone();
        let defaults = CaptionStyle::default();
        let mut changed = false;
        let spec = EffectsSpec {
            glow_picker: editor.picker.target == Some(CaptionColor::Glow),
            tune_pulse: editor.tune_target == Some(TuneTarget::Pulse),
            tune_hue: editor.tune_target == Some(TuneTarget::Hue),
        };
        let pane = layout::effects_layout(body, spec);
        let offset = self.scroll_body(d, input, editor, body, pane.content_height);
        let s = |rect: UiRect| layout::scrolled(rect, offset);
        let old_clip = self.widgets.set_input_clip(Some(body));
        let mut clip = widgets::begin_scissor(d, body, input.ui_scale);
        let d = &mut *clip;
        let mut report = Vec::new();

        // GLOW
        widgets::section_head(d, font, s(pane.glow_head), &[MicroRun::muted("Glow")]);
        #[rustfmt::skip]
        let glow_rows: [(LabelledRow, &str, f64, f64, u32, u8, &str); 2] = [
            (pane.glow,   "Glow",   caption_fx::GLOW_STRENGTH_MINIMUM, caption_fx::GLOW_STRENGTH_MAXIMUM, column_id::GLOW,   0,
             "Halo brightness; 0 turns the glow off"),
            (pane.radius, "Radius", caption_fx::GLOW_RADIUS_MINIMUM,   caption_fx::GLOW_RADIUS_MAXIMUM,   column_id::RADIUS, 1,
             "Halo size, as a fraction of the text size"),
        ];
        for (row, label, minimum, maximum, id, which, tip) in glow_rows {
            let (value, default) = match which {
                0 => (
                    &mut style.effects.glow_strength,
                    defaults.effects.glow_strength,
                ),
                _ => (&mut style.effects.glow_radius, defaults.effects.glow_radius),
            };
            if self
                .column_slider(
                    d,
                    font,
                    s(row.row),
                    label,
                    value,
                    default,
                    (minimum, maximum),
                    id,
                    0,
                    layout::PANE_LABEL,
                    tip,
                )
                .is_some()
            {
                changed = true;
            }
            report.push(format!(
                "{}@{}",
                label.to_lowercase(),
                rect_token(s(row.row))
            ));
        }
        let colour = LabelledRow {
            row: s(pane.colour.row),
            label: s(pane.colour.label),
            control: s(pane.colour.control),
        };
        pane_label(d, font, colour.label, "Colour");
        let swatch_rect = layout::swatch_rects(colour.control, 1)[0];
        let glow_id = widgets::widget_id(ns::CAPTION, caption_id::GLOW_COLOR);
        let open = editor.picker.target == Some(CaptionColor::Glow);
        let result = self.widgets.swatch(
            d,
            glow_id,
            swatch_rect,
            Color::get_color(style.effects.glow_rgba),
            open,
        );
        self.widgets.hint(
            d,
            result.state,
            glow_id,
            swatch_rect,
            "The glow's base colour; click for the free picker",
        );
        if result.state.clicked {
            editor.picker.toggle(CaptionColor::Glow);
            editor.tune_target = None;
        }
        let (red, green, blue, _) = widgets::unpack_rgba(style.effects.glow_rgba);
        widgets::draw_text_tabular(
            d,
            font,
            &format!("#{red:02X}{green:02X}{blue:02X}"),
            swatch_rect.x + layout::SWATCH + 10.0,
            (colour.row.y + (colour.row.height - metric::UI_FONT_CAPTION) * 0.5).round(),
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        report.push(format!("colour@{}", rect_token(swatch_rect)));
        if let Some(block) = pane.picker {
            if self.inline_picker(
                d,
                input,
                editor,
                s_picker(block, offset),
                CaptionColor::Glow,
                &mut style,
            ) {
                changed = true;
            }
        }

        // PULSE and HUE
        for (group, title, target, slider) in [
            (pane.pulse, "Pulse", TuneTarget::Pulse, 0u8),
            (pane.hue, "Hue", TuneTarget::Hue, 1),
        ] {
            widgets::section_head(d, font, s(group.head), &[MicroRun::muted(title)]);
            let current = target.drive(&style);
            let selected = index_of(EffectDrive::ALL, current);
            let first = if slider == 0 {
                caption_id::PULSE_DRIVE
            } else {
                caption_id::HUE_DRIVE
            };
            let drive = self.widgets.segmented(
                d,
                font,
                ns::CAPTION,
                first,
                s(group.drive),
                &DRIVE_LABELS,
                Some(selected),
                SegmentedFit::Fill,
            );
            if let Some(chosen) = drive.clicked {
                if let Some(value) = EffectDrive::ALL.get(chosen) {
                    if *value != current {
                        match target {
                            TuneTarget::Pulse => style.effects.glow_pulse = *value,
                            TuneTarget::Hue => style.effects.glow_hue_drive = *value,
                        }
                        changed = true;
                    }
                }
            }
            // A live meter under every source, so the music is seen moving
            // each one before it is chosen (§3.2). The chosen one in the
            // level colour, the rest muted.
            let meters = s(group.meters);
            let mut levels = Vec::new();
            for (index, item) in drive.layout.items.iter().enumerate() {
                let Some(source) = EffectDrive::ALL.get(index) else {
                    continue;
                };
                let level = drive_value(*source, &input.effect_inputs).clamp(0.0, 1.0);
                let track_rect = UiRect::new(
                    item.x + 4.0,
                    meters.y,
                    (item.width - 8.0).max(0.0),
                    meters.height,
                );
                widgets::fill(d, track_rect, color::ui_selected());
                if level > 0.0 {
                    widgets::fill(
                        d,
                        UiRect::new(
                            track_rect.x,
                            track_rect.y,
                            track_rect.width * level,
                            track_rect.height,
                        ),
                        if index == selected {
                            color::ui_level()
                        } else {
                            color::ui_muted()
                        },
                    );
                }
                levels.push(format!("{}={level:.2}", DRIVE_LABELS[index].to_lowercase()));
            }
            report.push(format!(
                "{}={}@{} meters={}",
                title.to_lowercase(),
                DRIVE_LABELS[selected].to_lowercase(),
                rect_token(s(group.drive)),
                levels.join(";")
            ));

            let row = s(group.amount.row);
            let tune_open = editor.tune_target == Some(target);
            let (label, value, default, range, id, tip, unit) = if slider == 0 {
                (
                    "Depth",
                    &mut style.effects.glow_pulse_depth,
                    defaults.effects.glow_pulse_depth,
                    (
                        caption_fx::PULSE_DEPTH_MINIMUM,
                        caption_fx::PULSE_DEPTH_MAXIMUM,
                    ),
                    column_id::DEPTH,
                    "How much of the glow the pulse owns: 0 steady, 100 % swings to nothing",
                    None,
                )
            } else {
                (
                    "Sweep",
                    &mut style.effects.glow_hue_range,
                    defaults.effects.glow_hue_range,
                    (caption_fx::HUE_RANGE_MINIMUM, caption_fx::HUE_RANGE_MAXIMUM),
                    column_id::SWEEP,
                    "How many degrees of hue the drive sweeps through at full value",
                    Some("\u{b0}"),
                )
            };
            let (minimum, maximum) = range;
            let span = (maximum - minimum).max(f64::EPSILON);
            let value_text = match unit {
                Some(unit) => format!("{:.0}{unit}", *value),
                None => format!("{:.0} %", *value * 100.0),
            };
            let result = self.widgets.slider_row(
                d,
                font,
                ns::CAPTION,
                id,
                row,
                &SliderRow {
                    label,
                    value_text: &value_text,
                    normalized: ((*value - minimum) / span) as f32,
                    default: Some(((default - minimum) / span) as f32),
                    changed: (*value - default).abs() > 1e-9,
                    route: Some(tune_open),
                    label_width: layout::PANE_LABEL,
                },
            );
            if let Some(fraction) = result.drag {
                let next = (minimum + f64::from(fraction) * span).clamp(minimum, maximum);
                if next != *value {
                    *value = next;
                    changed = true;
                }
            }
            self.widgets.hint(
                d,
                result.track,
                widgets::widget_id(ns::CAPTION, id),
                row,
                tip,
            );
            let route_id = widgets::widget_id(ns::CAPTION, id + 3);
            self.widgets.hint(
                d,
                result.route,
                route_id,
                result.layout.route,
                "~ Tune: shape how this drive answers the music",
            );
            if result.route.clicked {
                editor.tune_target = if tune_open { None } else { Some(target) };
                editor.picker.target = None;
            }
            report.push(format!(
                "{}@{} tune@{}",
                label.to_lowercase(),
                rect_token(row),
                rect_token(result.layout.route)
            ));
            if let Some(tune) = group.tune {
                let tune = s_tune(tune, offset);
                widgets::rounded_box(
                    d,
                    tune.card,
                    layout::CARD_RADIUS,
                    color::ui_raised(),
                    color::ui_selected_edge(),
                    1.0,
                );
                let band = CaptionTuneLayout {
                    heading: tune.heading,
                    clamp: tune.clamp,
                    reset: tune.reset,
                    meter_caption: tune.meter_caption,
                    meter: tune.meter,
                    quiet_y: tune.quiet_y,
                    loud_y: tune.loud_y,
                    curve_y: tune.curve_y,
                    x: tune.x,
                    width: tune.width,
                };
                changed |= self.caption_tune_editor(d, input, &band, target, &mut style);
                let tuning = target.read(&style);
                let live = f64::from(drive_value(target.drive(&style), &input.effect_inputs));
                mapping_editor::transfer_graph(
                    d,
                    tune.graph,
                    (tuning.input_min, tuning.input_max),
                    0.0,
                    1.0,
                    Some(live),
                    &|source| Some(tuning.apply(source)),
                );
                report.push(format!("editor@{}", rect_token(tune.card)));
            }
        }
        draw_note(
            d,
            font,
            s(pane.note),
            "Drives follow the music; ~ Tune shapes the response. Backing softness lives on Style.",
        );
        drop(clip);
        self.widgets.set_input_clip(old_clip);
        if changed {
            editor.push(LyricsEdit::SetCaptionStyle(Box::new(style)));
        }
        format!(
            "pane=effects {} picker={} tune={} scroll={:.0}/{:.0}",
            report.join(" "),
            editor.picker.target.map_or("none", |target| target.token()),
            editor.tune_target.map_or("none", |target| target.token()),
            offset,
            layout::max_scroll(pane.content_height, body),
        )
    }

    // -----------------------------------------------------------------------
    // Pieces
    // -----------------------------------------------------------------------

    /// The pane's scroll offset after this frame's wheel, clamped to the
    /// content. A slider drag in progress keeps the surface still under it.
    fn scroll_body(
        &mut self,
        d: &RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        body: UiRect,
        content_height: f32,
    ) -> f32 {
        let mouse = input.ui_scale.mouse(d);
        if body.contains_point(mouse.x, mouse.y) {
            let wheel = self.wheel_delta(d);
            if wheel != 0.0 {
                editor.column.scroll -= wheel * 40.0;
            }
        }
        editor.column.scroll = editor
            .column
            .scroll
            .clamp(0.0, layout::max_scroll(content_height, body));
        editor.column.scroll
    }

    /// A labelled parameter row through the shared `slider_row`: label, the
    /// track with a tick at the default, the value. Returns the row when the
    /// value moved.
    #[allow(
        clippy::too_many_arguments,
        reason = "a slider row's content plus the range, its ids and its tip"
    )]
    fn column_slider(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &UiFonts,
        row: UiRect,
        label: &str,
        value: &mut f64,
        default: f64,
        (minimum, maximum): (f64, f64),
        first_index: u32,
        precision: usize,
        label_width: f32,
        tip: &str,
    ) -> Option<UiRect> {
        let span = (maximum - minimum).max(f64::EPSILON);
        let value_text = format!("{:.precision$} %", *value * 100.0);
        let result = self.widgets.slider_row(
            d,
            font,
            ns::CAPTION,
            first_index,
            row,
            &SliderRow {
                label,
                value_text: &value_text,
                normalized: ((*value - minimum) / span) as f32,
                default: Some(((default - minimum) / span) as f32),
                changed: (*value - default).abs() > 1e-9,
                route: None,
                label_width,
            },
        );
        self.widgets.hint(
            d,
            result.track,
            widgets::widget_id(ns::CAPTION, first_index),
            result.layout.track,
            tip,
        );
        let fraction = result.drag?;
        let next = (minimum + f64::from(fraction) * span).clamp(minimum, maximum);
        if next == *value {
            return None;
        }
        *value = next;
        Some(row)
    }

    /// The free picker in a raised card under its row: the band's picker
    /// controls, placed by the column's layout.
    fn inline_picker(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        editor: &mut LyricEditor,
        block: PickerLayout,
        target: CaptionColor,
        style: &mut CaptionStyle,
    ) -> bool {
        widgets::rounded_box(
            d,
            block.card,
            layout::CARD_RADIUS,
            color::ui_raised(),
            color::ui_selected_edge(),
            1.0,
        );
        let band = CaptionPickerLayout {
            heading: UiRect::new(
                block.heading.x,
                block.heading.y + 5.0,
                block.heading.width,
                12.0,
            ),
            done: block.done,
            field: block.field,
            hue: block.hue,
            alpha: block.alpha,
            readout: block.readout,
        };
        self.caption_picker(
            d,
            input.fonts.ui(),
            input.ui_scale,
            editor,
            &band,
            target,
            style,
        )
    }

    /// A ghost button carrying one icon, or `fallback` in words when the icon
    /// face did not load (`Faces::icons_available`).
    #[allow(
        clippy::too_many_arguments,
        reason = "a button's arguments plus its two faces"
    )]
    fn ghost_icon(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        id: u64,
        rect: UiRect,
        icon: Icon,
        fallback: &str,
        selected: bool,
    ) -> widgets::ButtonState {
        if fonts.icons_available() {
            self.widgets.icon_button(
                d,
                fonts.icons(),
                id,
                rect,
                &icon.glyph().to_string(),
                selected,
                ButtonStyle::Ghost,
                13.0,
            )
        } else {
            self.widgets.text_button(
                d,
                fonts.ui(),
                id,
                rect,
                fallback,
                selected,
                ButtonStyle::Ghost,
                Some(metric::UI_FONT_CAPTION),
            )
        }
    }

    /// [`Self::ghost_icon`] that cannot be pressed: the glyph in the disabled
    /// ink, no claim.
    fn disabled_icon(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        rect: UiRect,
        icon: Icon,
        fallback: &str,
    ) {
        self.icon_glyph(d, fonts, rect, icon, fallback, color::ui_disabled());
    }

    /// One icon centred in `rect`, drawn and not pressable.
    fn icon_glyph(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        rect: UiRect,
        icon: Icon,
        fallback: &str,
        tint: Color,
    ) {
        if fonts.icons_available() {
            let glyph = icon.glyph().to_string();
            let face = fonts.icons();
            let size = 13.0;
            let measured = face.measure_text(&glyph, size, 0.0);
            d.draw_text_ex(
                face,
                &glyph,
                Vector2::new(
                    (rect.x + (rect.width - measured.x) * 0.5).round(),
                    (rect.y + (rect.height - measured.y) * 0.5).round(),
                ),
                size,
                0.0,
                tint,
            );
        } else {
            widgets::text_label(d, fonts.ui(), rect, fallback, metric::UI_FONT_CAPTION, tint);
        }
    }

    /// A button with an icon and words: `+ Add a cue at 01:35.000`.
    #[allow(
        clippy::too_many_arguments,
        reason = "a button's arguments plus its icon"
    )]
    fn icon_word_button(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        id: u64,
        rect: UiRect,
        icon: Icon,
        label: &str,
        style: ButtonStyle,
    ) -> widgets::ButtonState {
        let state = self.widgets.button(d, id, rect);
        let look = widgets::button_look(style, false, state.hovered);
        widgets::control_box(
            d,
            rect,
            look.fill,
            look.edge,
            if state.pressed { 2.0 } else { 1.0 },
        );
        let font = fonts.ui();
        let words = widgets::measure(font, label, metric::UI_FONT_CAPTION);
        let icon_width = if fonts.icons_available() { 18.0 } else { 0.0 };
        let left = (rect.x + (rect.width - words - icon_width) * 0.5).round();
        if fonts.icons_available() {
            self.icon_glyph(
                d,
                fonts,
                UiRect::new(left, rect.y, 12.0, rect.height),
                icon,
                "",
                look.ink,
            );
        }
        widgets::draw_text(
            d,
            font,
            label,
            left + icon_width,
            (rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5).round(),
            metric::UI_FONT_CAPTION,
            look.ink,
        );
        state
    }
}

/// A picker layout moved by the pane's scroll.
fn s_picker(block: PickerLayout, offset: f32) -> PickerLayout {
    PickerLayout {
        card: layout::scrolled(block.card, offset),
        heading: layout::scrolled(block.heading, offset),
        done: layout::scrolled(block.done, offset),
        field: layout::scrolled(block.field, offset),
        hue: layout::scrolled(block.hue, offset),
        alpha: layout::scrolled(block.alpha, offset),
        readout: layout::scrolled(block.readout, offset),
    }
}

/// A tune layout moved by the pane's scroll.
fn s_tune(tune: TuneLayout, offset: f32) -> TuneLayout {
    TuneLayout {
        card: layout::scrolled(tune.card, offset),
        heading: layout::scrolled(tune.heading, offset),
        clamp: layout::scrolled(tune.clamp, offset),
        reset: layout::scrolled(tune.reset, offset),
        meter_caption: layout::scrolled(tune.meter_caption, offset),
        meter: layout::scrolled(tune.meter, offset),
        quiet_y: tune.quiet_y - offset,
        loud_y: tune.loud_y - offset,
        curve_y: tune.curve_y - offset,
        graph: layout::scrolled(tune.graph, offset),
        x: tune.x,
        width: tune.width,
    }
}

/// A pane row's 13 px muted label, vertically centred.
fn pane_label(d: &mut RaylibDrawHandle<'_>, font: &UiFonts, rect: UiRect, text: &str) {
    widgets::draw_text(
        d,
        font,
        text,
        rect.x,
        (rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5).round(),
        metric::UI_FONT_CAPTION,
        color::ui_muted(),
    );
}

/// The closing note: micro type, wrapped to the pane.
fn draw_note(d: &mut RaylibDrawHandle<'_>, font: &UiFonts, rect: UiRect, text: &str) {
    let measure = |line: &str| widgets::micro_width(font, &[MicroRun::muted(line)]);
    let lines = musializer_core::ui::notice::wrap_detail(text, rect.width, 2, measure);
    for (index, line) in lines.iter().enumerate() {
        widgets::draw_micro(
            d,
            font,
            rect.x,
            rect.y + index as f32 * 17.0,
            &[MicroRun::muted(line)],
        );
    }
}

/// The legend's words with their counts: `USER 83`, `CERTAIN`, …
fn legend_entries(document: &LyricsDocument) -> Vec<(CueOrigin, String)> {
    ORIGIN_ORDER
        .iter()
        .map(|origin| {
            let count = document
                .cues()
                .iter()
                .filter(|cue| cue.origin == *origin)
                .count();
            let word = origin_legend_word(*origin);
            (
                *origin,
                if count > 0 {
                    format!("{word} {count}")
                } else {
                    word.to_owned()
                },
            )
        })
        .collect()
}

const LEGEND_DOT_GAP: f32 = 5.0;
const LEGEND_ITEM_GAP: f32 = 12.0;

/// Width of the legend laid in one row.
fn legend_width(font: &UiFonts, document: &LyricsDocument, _narrow: bool) -> f32 {
    let entries = legend_entries(document);
    entries
        .iter()
        .map(|(_, word)| {
            widgets::STATUS_DOT_SIZE
                + LEGEND_DOT_GAP
                + widgets::micro_width(font, &[MicroRun::muted(word)])
        })
        .sum::<f32>()
        + LEGEND_ITEM_GAP * (entries.len().saturating_sub(1)) as f32
}

/// The origin key: a dot in each origin's colour and its micro word, in one
/// row or two.
fn draw_legend(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    document: &LyricsDocument,
    rect: UiRect,
    rows: usize,
) {
    let entries = legend_entries(document);
    let per_row = entries.len().div_ceil(rows.max(1));
    for (row_index, chunk) in entries.chunks(per_row.max(1)).enumerate() {
        let y = rect.y + row_index as f32 * (layout::LEGEND_ROW + layout::LEGEND_ROW_GAP);
        let mut x = rect.x;
        for (origin, word) in chunk {
            let runs = [MicroRun::muted(word)];
            let width = widgets::micro_width(font, &runs);
            if x + widgets::STATUS_DOT_SIZE + LEGEND_DOT_GAP + width > rect.x + rect.width + 0.5 {
                break;
            }
            d.draw_circle_v(
                Vector2::new(
                    x + widgets::STATUS_DOT_SIZE * 0.5,
                    y + layout::LEGEND_ROW * 0.5,
                ),
                widgets::STATUS_DOT_SIZE * 0.5,
                origin_color(*origin),
            );
            x += widgets::STATUS_DOT_SIZE + LEGEND_DOT_GAP;
            widgets::draw_micro(
                d,
                font,
                x,
                (y + (layout::LEGEND_ROW - metric::UI_FONT_MICRO) * 0.5).round(),
                &runs,
            );
            x += width + LEGEND_ITEM_GAP;
        }
    }
}

/// A dashed outline round `rect` (the `+ Import` chip, `.btn.dashed`).
fn dashed_outline(d: &mut RaylibDrawHandle<'_>, rect: UiRect, tint: Color) {
    const DASH: f32 = 4.0;
    const GAP: f32 = 3.0;
    let mut x = rect.x;
    while x < rect.x + rect.width {
        let w = DASH.min(rect.x + rect.width - x);
        widgets::fill(d, UiRect::new(x, rect.y, w, 1.0), tint);
        widgets::fill(d, UiRect::new(x, rect.y + rect.height - 1.0, w, 1.0), tint);
        x += DASH + GAP;
    }
    let mut y = rect.y;
    while y < rect.y + rect.height {
        let h = DASH.min(rect.y + rect.height - y);
        widgets::fill(d, UiRect::new(rect.x, y, 1.0, h), tint);
        widgets::fill(d, UiRect::new(rect.x + rect.width - 1.0, y, 1.0, h), tint);
        y += DASH + GAP;
    }
}

/// A dashed circle (the swatch row's `+`).
fn dashed_circle(d: &mut RaylibDrawHandle<'_>, centre: Vector2, radius: f32, tint: Color) {
    for step in 0..12 {
        let start = step as f32 * 30.0;
        d.draw_ring(centre, radius - 1.0, radius, start, start + 18.0, 4, tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pane_is_one_value_over_the_three_flags_and_round_trips() {
        let mut editor = LyricEditor::new();
        for pane in [
            LyricsPane::Cues,
            LyricsPane::Style,
            LyricsPane::Effects,
            LyricsPane::Fonts,
            LyricsPane::Cues,
        ] {
            editor.set_pane(pane);
            assert_eq!(editor.pane(), pane);
        }
        // The band's own precedence: the browser wins over Effects.
        editor.style_pane = true;
        editor.effects_pane = true;
        editor.font_pane = true;
        assert_eq!(editor.pane(), LyricsPane::Fonts);
    }

    #[test]
    fn leaving_a_pane_closes_its_disclosures_and_the_cue_field() {
        let mut editor = LyricEditor::new();
        editor.set_pane(LyricsPane::Effects);
        editor.tune_target = Some(TuneTarget::Hue);
        editor.picker.toggle(CaptionColor::Glow);
        editor.column.scroll = 120.0;
        editor.set_pane(LyricsPane::Style);
        assert_eq!(editor.tune_target, None);
        assert_eq!(editor.picker.target, None);
        assert_eq!(editor.column.scroll, 0.0);
        editor.set_pane(LyricsPane::Cues);
        editor.text.set_focused(true);
        editor.set_pane(LyricsPane::Style);
        assert!(!editor.text.is_focused());
        assert!(LyricsPane::Style.shapes_caption() && !LyricsPane::Cues.shapes_caption());
    }

    #[test]
    fn the_grid_cells_name_the_anchor_they_choose() {
        // Top centre is cell 1 and the enum's index 7 (it counts from the
        // bottom row), bottom centre cell 7 and index 1 — the default.
        assert_eq!(anchor_value(1), 7);
        assert_eq!(anchor_value(7), 1);
        assert_eq!(ANCHOR_WORDS[7], "bottom centre");
        let mut seen: Vec<usize> = (0..9).map(anchor_value).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn the_columns_widget_indices_stay_clear_of_the_bands() {
        // The band's ACTIONS indices are 0..=8; its CAPTION ones end in the
        // zone hints, which take a dozen slots from 160.
        for index in [
            action::PANES,
            action::PANES + 2,
            action::BACK_TO_STYLE,
            action::PREVIOUS,
            action::NEXT,
            action::FOLLOW,
            action::PLAYHEAD_WORD,
            action::PLAYHEAD_WORD + 1,
        ] {
            assert!(index > 8, "{index}");
        }
        let mut caption = vec![
            column_id::SIZE,
            column_id::WIDTH,
            column_id::BLUR,
            column_id::SHADE,
            column_id::ROUND,
            column_id::INSET,
            column_id::GLOW,
            column_id::RADIUS,
            column_id::DEPTH,
            column_id::SWEEP,
        ];
        for pair in caption.windows(2) {
            assert!(pair[1] - pair[0] >= widgets::SLIDER_ROW_IDS, "{pair:?}");
        }
        assert!(column_id::SIZE >= caption_id::ZONE_HINTS + 16);
        caption.extend([
            column_id::INK_PLUS,
            column_id::PLATE_PLUS,
            column_id::IMPORT_CANCEL,
        ]);
        caption.sort_unstable();
        caption.dedup();
        assert_eq!(caption.len(), 13);
        assert!(column_id::INK_PLUS >= column_id::SWEEP + widgets::SLIDER_ROW_IDS);
    }
}
