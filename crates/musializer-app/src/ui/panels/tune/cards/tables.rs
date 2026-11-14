//! The captions card's tables (D, slice 3): the voices, the named caption
//! styles and the title cards, each edited row by row and written back as a
//! whole table across the plan boundary, where [`apply_table_edit`]
//! validates it the way a loader would.
use super::*;
use musializer_core::project::caption_styles::{
    self, CaptionOverride, NamedCaptionStyle, TitleCard,
};
use musializer_core::project::lyrics::{Voice, VOICE_CAPACITY};
use musializer_core::project::model::CaptionAnchor;

/// One row of a table, by index into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TablePart {
    VoicesLabel,
    VoicesNone,
    VoiceName(usize),
    VoiceHue(usize),
    VoiceInk(usize),
    VoiceGlow(usize),
    AddVoice,
    StylesLabel,
    StyleName(usize),
    StyleAnchor(usize),
    StyleSize(usize),
    StyleInk(usize),
    AddStyle,
    TitlesLabel,
    TitlesNone,
    TitleText(usize),
    TitleArtist(usize),
    TitleHandle(usize),
    TitleTimes(usize),
    TitleStyle(usize),
    AddTitle,
}

/// What a table row draws with: the track it reads, where it goes, and the
/// lists it adds its report and its edits to.
pub(super) struct Context<'a> {
    pub track: &'a Track,
    pub place: &'a Placed,
    pub enabled: bool,
    pub label_width: f32,
    pub time_seconds: f64,
    pub controls: &'a mut Vec<String>,
    pub edits: &'a mut Vec<ScenePlanEdit>,
}

/// Slots in `LOOK_CARDS`: thirty per row of a table.
mod table_slot {
    pub const ADD_VOICE: u32 = 395;
    pub const ADD_STYLE: u32 = 396;
    pub const ADD_TITLE: u32 = 397;
    pub const VOICE: u32 = 400;
    pub const STYLE: u32 = 700;
    pub const TITLE: u32 = 1200;
    pub const STRIDE: u32 = 30;
}

/// The nine anchors as the grid shows them: top row first, as on screen.
const ANCHORS: [(CaptionAnchor, &str); 9] = [
    (CaptionAnchor::TopLeft, "Top left"),
    (CaptionAnchor::TopCenter, "Top"),
    (CaptionAnchor::TopRight, "Top right"),
    (CaptionAnchor::MiddleLeft, "Left"),
    (CaptionAnchor::MiddleCenter, "Middle"),
    (CaptionAnchor::MiddleRight, "Right"),
    (CaptionAnchor::BottomLeft, "Bottom left"),
    (CaptionAnchor::BottomCenter, "Bottom"),
    (CaptionAnchor::BottomRight, "Bottom right"),
];

/// Inks a voice or a style can take: light, so they read over any scene.
const INKS: [u32; 6] = [0xffffff, 0xffe8c0, 0xe9e3ff, 0xc8f0ff, 0xffd0e0, 0xd8ffd0];
/// Glows: saturated versions of the same.
const GLOWS: [u32; 6] = [0xffcf86, 0xb9a6ff, 0x7fd8ff, 0xff86b0, 0x9cff86, 0xffffff];

/// Title text, artist and handle take any printable text up to the model's
/// limit; names are ASCII.
const TITLE_RULES: TextRules = TextRules {
    max_bytes: caption_styles::TITLE_TEXT_MAX_BYTES,
    ascii_only: false,
};

/// How many lines, sections and cards name style `name`.
fn style_uses(track: &Track, name: caption_styles::StyleName) -> (usize, usize, usize) {
    (
        track
            .lyrics
            .cue_styles()
            .filter(|(_, used)| *used == name)
            .count(),
        track
            .scene_switches
            .cues()
            .iter()
            .filter(|cue| cue.caption_style == Some(name))
            .count(),
        track
            .scene_switches
            .titles
            .iter()
            .filter(|card| card.style == Some(name))
            .count(),
    )
}

/// The tables' parts, appended to the captions card.
pub(super) fn plan(track: &Track, parts: &mut Vec<(Part, Block)>) {
    let row = |part| (Part::Table(part), Block::Field);
    let gap = (Part::Space, Block::Gap(look_cards::GROUP_GAP));
    let small = (Part::Space, Block::Gap(6.0));
    parts.push((Part::Table(TablePart::VoicesLabel), Block::Label));
    let voices = track.lyrics.voices().len();
    if voices == 0 {
        parts.push((Part::Table(TablePart::VoicesNone), Block::Lines(2)));
    }
    for i in 0..voices {
        parts.extend([
            row(TablePart::VoiceName(i)),
            (Part::Table(TablePart::VoiceHue(i)), Block::Slider),
            (Part::Table(TablePart::VoiceInk(i)), Block::Slider),
            (Part::Table(TablePart::VoiceGlow(i)), Block::Slider),
            small,
        ]);
    }
    parts.extend([(Part::Table(TablePart::AddVoice), Block::Buttons), gap]);
    parts.push((Part::Table(TablePart::StylesLabel), Block::Label));
    for i in 0..track.caption_style.styles.len() {
        parts.extend([
            row(TablePart::StyleName(i)),
            (Part::Table(TablePart::StyleAnchor(i)), Block::Columns(9, 3)),
            (Part::Space, Block::Gap(4.0)),
            (Part::Table(TablePart::StyleSize(i)), Block::Slider),
            (Part::Table(TablePart::StyleInk(i)), Block::Slider),
            small,
        ]);
    }
    parts.extend([(Part::Table(TablePart::AddStyle), Block::Buttons), gap]);
    parts.push((Part::Table(TablePart::TitlesLabel), Block::Label));
    let titles = &track.scene_switches.titles;
    if titles.is_empty() {
        parts.push((Part::Table(TablePart::TitlesNone), Block::Lines(2)));
    }
    let styles = track.caption_style.styles.len();
    for i in 0..titles.len() {
        parts.extend([
            row(TablePart::TitleText(i)),
            (Part::Space, Block::Gap(4.0)),
            row(TablePart::TitleArtist(i)),
            (Part::Space, Block::Gap(4.0)),
            row(TablePart::TitleHandle(i)),
            (Part::Space, Block::Gap(4.0)),
            (Part::Table(TablePart::TitleTimes(i)), Block::Buttons),
            (Part::Space, Block::Gap(4.0)),
            (
                Part::Table(TablePart::TitleStyle(i)),
                Block::Grid(1 + styles),
            ),
            small,
        ]);
    }
    parts.push((Part::Table(TablePart::AddTitle), Block::Buttons));
}

/// Applies a table edit to `track` as a whole, or changes nothing: the same
/// rules a loader holds a project to (names valid and unique, every name a
/// line, section or card uses in the table, cards in order and apart). A
/// renamed style takes every line, section and card that named it along.
pub(crate) fn apply_table_edit(track: &mut Track, edit: ScenePlanEdit) -> bool {
    match edit {
        ScenePlanEdit::SetVoices(voices) => track.lyrics.set_voices(voices),
        ScenePlanEdit::RemoveVoice(index) => track.lyrics.remove_voice(index),
        ScenePlanEdit::SetCaptionStyles { table, renamed } => {
            if !caption_styles::table_is_valid(&table) {
                return false;
            }
            let mut lyrics = track.lyrics.clone();
            let mut plan = track.scene_switches.clone();
            if let Some((from, to)) = renamed {
                let lines: Vec<u64> = lyrics
                    .cue_styles()
                    .filter(|(_, name)| *name == from)
                    .map(|(id, _)| id)
                    .collect();
                for id in lines {
                    lyrics.set_cue_style(id, Some(to));
                }
                let sections: Vec<u64> = plan
                    .cues()
                    .iter()
                    .filter(|cue| cue.caption_style == Some(from))
                    .map(|cue| cue.id)
                    .collect();
                for id in sections {
                    plan.set_caption_style(id, Some(to));
                }
                for card in &mut plan.titles {
                    if card.style == Some(from) {
                        card.style = Some(to);
                    }
                }
            }
            let known =
                |name: caption_styles::StyleName| caption_styles::find(&table, name).is_some();
            let used_known = lyrics.cue_styles().all(|(_, name)| known(name))
                && plan
                    .cues()
                    .iter()
                    .all(|cue| cue.caption_style.is_none_or(known))
                && caption_styles::titles_are_valid(&plan.titles, track.duration_seconds, &table);
            if !used_known {
                return false;
            }
            track.caption_style.styles = table;
            track.lyrics = lyrics;
            track.scene_switches = plan;
            true
        }
        ScenePlanEdit::SetTitles(titles) => {
            if !caption_styles::titles_are_valid(
                &titles,
                track.duration_seconds,
                &track.caption_style.styles,
            ) {
                return false;
            }
            track.scene_switches.titles = titles;
            true
        }
        ScenePlanEdit::SetBeyond { cue_id, beyond } => {
            use musializer_core::scene::settings::index::threshold::BEYOND;
            let scene = SceneId::Threshold;
            let Some(cue) = track
                .scene_switches
                .cues()
                .iter()
                .find(|cue| cue.id == cue_id)
            else {
                return false;
            };
            if cue.scene_index != scene.index() as u32 {
                return false;
            }
            // The section's tuning as it plays: the track's table with its
            // own snapshot over it, then the one value changed.
            let mut table = track.scene_settings;
            if cue.settings.captured {
                table.apply_snapshot(scene, &cue.settings);
            }
            if !table.set(scene, BEYOND, beyond) {
                return false;
            }
            table
                .capture(scene)
                .is_some_and(|snapshot| track.scene_switches.set_settings(cue_id, snapshot))
        }
        _ => false,
    }
}

fn packed_rgb(rgba: Option<u32>) -> Option<u32> {
    rgba.map(|rgba| rgba >> 8)
}

fn packed_rgba(rgb: u32) -> u32 {
    (rgb << 8) | 0xff
}

/// A unique `stem-N` name not yet in `taken`.
fn fresh_name(stem: &str, taken: &[String]) -> String {
    (1..)
        .map(|n| format!("{stem}-{n}"))
        .find(|name| !taken.contains(name))
        .unwrap_or_else(|| stem.to_owned())
}

impl Shell {
    /// A text row: a box that shows the value (or the placeholder, muted)
    /// and types on a press, and an optional button at its right. Returns the
    /// committed text and whether the button was pressed.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row: its field, value, rules, button and report"
    )]
    fn text_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        rect: UiRect,
        field: CardField,
        first: u32,
        value: &str,
        placeholder: &str,
        rules: TextRules,
        button: Option<(&str, Option<&str>)>,
        enabled: bool,
        controls: &mut Vec<String>,
    ) -> (Option<String>, bool) {
        let mut pressed = false;
        let mut boxed = rect;
        // The button and, when it cannot be pressed, the reason it says.
        if let Some((label, refusal)) = button {
            let width = (widgets::measure(font, label, metric::UI_FONT_CAPTION) + 18.0).ceil();
            let at = UiRect::new(
                rect.x + rect.width - width,
                rect.y + 1.0,
                width,
                rect.height - 2.0,
            );
            boxed.width = (at.x - 6.0 - rect.x).max(0.0);
            let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + 1);
            let armed = label.starts_with("Confirm");
            let state = if enabled && refusal.is_none() {
                self.widgets.text_button(
                    d,
                    font,
                    id,
                    at,
                    label,
                    false,
                    if armed {
                        widgets::ButtonStyle::Danger
                    } else {
                        widgets::ButtonStyle::Ghost
                    },
                    Some(metric::UI_FONT_CAPTION),
                )
            } else {
                self.widgets
                    .disabled_button(d, font, at, label, Some(metric::UI_FONT_CAPTION));
                let state = self.widgets.button(d, id, at);
                self.widgets
                    .hint(d, state, id, at, refusal.unwrap_or(FOLLOWING));
                widgets::ButtonState::default()
            };
            controls.push(format!(
                "{}-button@{}{}",
                field.name(),
                if enabled && refusal.is_none() {
                    ""
                } else {
                    "disabled:"
                },
                rect_text(at)
            ));
            pressed = state.clicked;
        }
        let id = widgets::widget_id(widgets::id::LOOK_CARDS, first);
        let state = self.widgets.button(d, id, boxed);
        let typing = matches!(&self.look_column.cards.typing, Some((open, _)) if *open == field);
        if !typing {
            widgets::control_box(
                d,
                boxed,
                if enabled && state.hovered {
                    color::ui_hover()
                } else {
                    color::ui_control()
                },
                color::ui_control_edge(),
                1.0,
            );
            let (text, ink) = if value.is_empty() {
                (placeholder, color::ui_disabled())
            } else {
                (
                    value,
                    if enabled {
                        color::ui_ink()
                    } else {
                        color::ui_disabled()
                    },
                )
            };
            widgets::draw_text_faded(
                d,
                font,
                text,
                boxed.x + 8.0,
                boxed.y + (boxed.height - metric::UI_FONT_CAPTION) * 0.5,
                (boxed.width - 16.0).max(0.0),
                16.0,
                metric::UI_FONT_CAPTION,
                ink,
            );
        }
        controls.push(format!("{}@{}", field.name(), rect_text(boxed)));
        if enabled && state.clicked && !typing {
            self.open_card_field_with(field, value, rules);
        }
        let committed = self.card_field(d, font, field, boxed);
        (committed.filter(|_| enabled), pressed)
    }

    /// The two-press arm for a removal that loses something: the first
    /// press arms, the second removes. Returns whether this press removes.
    fn arm_removal(&mut self, kind: u8, index: usize, pressed: bool, needs_confirm: bool) -> bool {
        if !pressed {
            return false;
        }
        if !needs_confirm || self.look_column.cards.armed == Some((kind, index)) {
            self.look_column.cards.armed = None;
            return true;
        }
        self.look_column.cards.armed = Some((kind, index));
        false
    }

    fn removal_label(&self, kind: u8, index: usize) -> &'static str {
        if self.look_column.cards.armed == Some((kind, index)) {
            "Confirm remove"
        } else {
            "Remove"
        }
    }

    /// One row of a table.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per kind of row; each is short"
    )]
    pub(super) fn table_part(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        row: TablePart,
        cx: &mut Context<'_>,
    ) {
        let track = cx.track;
        let rect = cx.place.rect;
        let enabled = cx.enabled;
        let voices = track.lyrics.voices();
        let styles = &track.caption_style.styles;
        let titles = &track.scene_switches.titles;
        let following = (!enabled).then_some(FOLLOWING);
        match row {
            TablePart::VoicesLabel => Self::card_label(
                d,
                font,
                rect,
                "Voices",
                &format!("{} of {VOICE_CAPACITY}", voices.len()),
            ),
            TablePart::VoicesNone => draw_wrapped(
                d,
                font,
                rect,
                "No voices. A duet reads better when each singer has a colour: a treatment names them and gives each line its voice.",
                color::ui_muted(),
            ),
            TablePart::VoiceName(i) => {
                let Some(voice) = voices.get(i) else { return };
                let label = self.removal_label(0, i);
                let uses = track.lyrics.voice_uses(i);
                let first = table_slot::VOICE + i as u32 * table_slot::STRIDE;
                let (typed, remove) = self.text_row(
                    d,
                    font,
                    rect,
                    CardField::VoiceName(i as u8),
                    first,
                    &voice.name,
                    "name",
                    TextRules::ascii_query(musializer_core::project::lyrics::VOICE_NAME_MAX_BYTES),
                    Some((label, None)),
                    enabled,
                    cx.controls,
                );
                if let Some(name) = typed.filter(|name| *name != voice.name && Voice::name_is_valid(name)) {
                    let mut next = voices.to_vec();
                    next[i].name = name;
                    cx.edits.push(ScenePlanEdit::SetVoices(next));
                }
                if self.arm_removal(0, i, remove, uses > 0) {
                    cx.edits.push(ScenePlanEdit::RemoveVoice(i));
                }
                if self.look_column.cards.armed == Some((0, i)) {
                    cx.controls.push(format!("voice-{i}-armed:{uses}-lines"));
                }
            }
            TablePart::VoiceHue(i) => {
                let Some(voice) = voices.get(i) else { return };
                let knob = Knob {
                    label: "Hue",
                    field: CardField::VoiceHue(i as u8),
                    first: table_slot::VOICE + i as u32 * table_slot::STRIDE + 2,
                    value: voice.style.hue.unwrap_or(0.0),
                    low: 0.0,
                    high: 360.0,
                    default: 0.0,
                    unit: Unit::Degrees,
                    scale: Scale::Linear,
                };
                if let Some(hue) = self.knob(d, font, rect, &knob, following, cx.label_width, cx.controls) {
                    let mut next = voices.to_vec();
                    next[i].style.hue = Some(hue);
                    cx.edits.push(ScenePlanEdit::SetVoices(next));
                }
            }
            TablePart::VoiceInk(i) | TablePart::VoiceGlow(i) => {
                let Some(voice) = voices.get(i) else { return };
                let ink = matches!(row, TablePart::VoiceInk(_));
                let first = table_slot::VOICE + i as u32 * table_slot::STRIDE + if ink { 6 } else { 15 };
                let current = if ink { voice.style.text_rgba } else { voice.style.glow_rgba };
                if let Some(rgb) = self.colour_row(
                    d,
                    font,
                    rect,
                    if ink { "Ink" } else { "Glow" },
                    if ink { CardField::VoiceInk(i as u8) } else { CardField::VoiceGlow(i as u8) },
                    first,
                    packed_rgb(current),
                    if ink { &INKS } else { &GLOWS },
                    enabled,
                    cx.label_width,
                    cx.controls,
                ) {
                    let mut next = voices.to_vec();
                    if ink {
                        next[i].style.text_rgba = Some(packed_rgba(rgb));
                    } else {
                        next[i].style.glow_rgba = Some(packed_rgba(rgb));
                    }
                    cx.edits.push(ScenePlanEdit::SetVoices(next));
                }
            }
            TablePart::AddVoice => {
                // A voice nothing sings draws nothing, and lines are given
                // their voice in a treatment: Add is shown, with why it waits.
                let label = "+ Add a voice";
                let width = (widgets::measure(font, label, metric::UI_FONT_CAPTION) + 24.0).ceil();
                let button = UiRect::new(rect.x, rect.y, width.min(rect.width), rect.height);
                self.widgets.disabled_button(d, font, button, label, Some(metric::UI_FONT_CAPTION));
                let id = widgets::widget_id(widgets::id::LOOK_CARDS, table_slot::ADD_VOICE);
                let state = self.widgets.button(d, id, button);
                self.widgets.hint(
                    d,
                    state,
                    id,
                    button,
                    "Not built here: a line is given its voice in a treatment (a line's \"voice\"), and a voice no line sings draws nothing. Edit the voices a treatment named above.",
                );
                cx.controls.push(format!("add-voice@disabled:{}", rect_text(button)));
            }
            TablePart::StylesLabel => Self::card_label(
                d,
                font,
                rect,
                "Caption styles",
                &format!("{} of {}", styles.len(), caption_styles::CAPTION_STYLE_CAPACITY),
            ),
            TablePart::StyleName(i) => {
                let Some(entry) = styles.get(i) else { return };
                let (lines, sections, cards) = style_uses(track, entry.name);
                let used = lines + sections + cards;
                let first = table_slot::STYLE + i as u32 * table_slot::STRIDE;
                let refusal = format!(
                    "In use by {lines} line(s), {sections} section(s) and {cards} title card(s): pick another style there first."
                );
                let (typed, remove) = self.text_row(
                    d,
                    font,
                    rect,
                    CardField::StyleName(i as u8),
                    first,
                    entry.name.as_str(),
                    "name",
                    TextRules::ascii_query(caption_styles::STYLE_NAME_MAX_BYTES),
                    Some(("Remove", (used > 0).then_some(refusal.as_str()))),
                    enabled,
                    cx.controls,
                );
                if let Some(name) = typed.and_then(|name| caption_styles::StyleName::new(&name)) {
                    if name != entry.name && caption_styles::find(styles, name).is_none() {
                        let mut table = styles.clone();
                        table[i].name = name;
                        cx.edits.push(ScenePlanEdit::SetCaptionStyles {
                            table,
                            renamed: Some((entry.name, name)),
                        });
                    }
                }
                if remove && used == 0 && enabled {
                    let mut table = styles.clone();
                    table.remove(i);
                    cx.edits.push(ScenePlanEdit::SetCaptionStyles { table, renamed: None });
                }
            }
            TablePart::StyleAnchor(i) => {
                let Some(entry) = styles.get(i) else { return };
                let first = table_slot::STYLE + i as u32 * table_slot::STRIDE + 2;
                for (k, ((anchor, label), chip)) in ANCHORS.iter().zip(&cx.place.chips).enumerate() {
                    let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + k as u32);
                    let selected = entry.style.anchor == Some(*anchor);
                    let state = self.card_chip(d, font, id, *chip, label, selected, enabled);
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        *chip,
                        if selected {
                            "Where this style puts a line; press again for the project's own place"
                        } else {
                            "Where this style puts a line"
                        },
                    );
                    cx.controls.push(format!("style-{i}-{}@{}", anchor.canonical_name(), rect_text(*chip)));
                    if state.clicked {
                        let mut table = styles.clone();
                        table[i].style.anchor = if selected { None } else { Some(*anchor) };
                        cx.edits.push(ScenePlanEdit::SetCaptionStyles { table, renamed: None });
                    }
                }
            }
            TablePart::StyleSize(i) => {
                let Some(entry) = styles.get(i) else { return };
                use musializer_core::project::model::caption;
                let base = track.caption_style.size_scale as f32;
                let knob = Knob {
                    label: "Size",
                    field: CardField::StyleSize(i as u8),
                    first: table_slot::STYLE + i as u32 * table_slot::STRIDE + 11,
                    value: entry.style.size_scale.map_or(base, |size| size as f32),
                    low: caption::SIZE_MINIMUM as f32,
                    high: caption::SIZE_MAXIMUM as f32,
                    default: base,
                    unit: Unit::ShortEdge,
                    scale: Scale::Linear,
                };
                if let Some(size) = self.knob(d, font, rect, &knob, following, cx.label_width, cx.controls) {
                    let mut table = styles.clone();
                    table[i].style.size_scale = Some(f64::from(size));
                    cx.edits.push(ScenePlanEdit::SetCaptionStyles { table, renamed: None });
                }
            }
            TablePart::StyleInk(i) => {
                let Some(entry) = styles.get(i) else { return };
                if let Some(rgb) = self.colour_row(
                    d,
                    font,
                    rect,
                    "Ink",
                    CardField::StyleInk(i as u8),
                    table_slot::STYLE + i as u32 * table_slot::STRIDE + 15,
                    packed_rgb(entry.style.text_rgba),
                    &INKS,
                    enabled,
                    cx.label_width,
                    cx.controls,
                ) {
                    let mut table = styles.clone();
                    table[i].style.text_rgba = Some(packed_rgba(rgb));
                    cx.edits.push(ScenePlanEdit::SetCaptionStyles { table, renamed: None });
                }
            }
            TablePart::AddStyle => {
                let full = styles.len() >= caption_styles::CAPTION_STYLE_CAPACITY;
                let label = "+ Add a style";
                let width = (widgets::measure(font, label, metric::UI_FONT_CAPTION) + 24.0).ceil();
                let button = UiRect::new(rect.x, rect.y, width.min(rect.width), rect.height);
                let id = widgets::widget_id(widgets::id::LOOK_CARDS, table_slot::ADD_STYLE);
                if enabled && !full {
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        button,
                        label,
                        false,
                        widgets::ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        button,
                        "A named style that states only what differs from the project's caption: sections, lines and title cards can pick it",
                    );
                    if state.clicked {
                        let taken: Vec<String> = styles.iter().map(|entry| entry.name.to_string()).collect();
                        if let Some(name) = caption_styles::StyleName::new(&fresh_name("style", &taken)) {
                            let mut table = styles.clone();
                            table.push(NamedCaptionStyle {
                                name,
                                style: CaptionOverride {
                                    anchor: Some(CaptionAnchor::TopCenter),
                                    ..CaptionOverride::default()
                                },
                            });
                            cx.edits.push(ScenePlanEdit::SetCaptionStyles { table, renamed: None });
                        }
                    }
                    cx.controls.push(format!("add-style@{}", rect_text(button)));
                } else {
                    self.widgets.disabled_button(d, font, button, label, Some(metric::UI_FONT_CAPTION));
                    cx.controls.push(format!("add-style@disabled:{}", rect_text(button)));
                }
            }
            TablePart::TitlesLabel => Self::card_label(
                d,
                font,
                rect,
                "Title cards",
                &format!("{} of {}", titles.len(), caption_styles::TITLE_CAPACITY),
            ),
            TablePart::TitlesNone => draw_wrapped(
                d,
                font,
                rect,
                "No title cards. One is added only when you ask for one, and it carries the artist.",
                color::ui_muted(),
            ),
            TablePart::TitleText(i) | TablePart::TitleArtist(i) | TablePart::TitleHandle(i) => {
                let Some(card) = titles.get(i) else { return };
                let first = table_slot::TITLE + i as u32 * table_slot::STRIDE;
                let (field, value, placeholder, button, offset) = match row {
                    TablePart::TitleText(_) => (
                        CardField::TitleText(i as u8),
                        card.title.clone(),
                        "title".to_owned(),
                        Some((self.removal_label(2, i), None)),
                        0,
                    ),
                    TablePart::TitleArtist(_) => (
                        CardField::TitleArtist(i as u8),
                        card.artist.clone().unwrap_or_default(),
                        track.project_artist().map_or_else(
                            || "artist (none: the card says NO ARTIST)".to_owned(),
                            |artist| format!("{artist} (the project's)"),
                        ),
                        None,
                        2,
                    ),
                    _ => (
                        CardField::TitleHandle(i as u8),
                        card.handle.clone().unwrap_or_default(),
                        "@handle".to_owned(),
                        None,
                        3,
                    ),
                };
                let (typed, remove) = self.text_row(
                    d,
                    font,
                    rect,
                    field,
                    first + offset,
                    &value,
                    &placeholder,
                    TITLE_RULES,
                    button,
                    enabled,
                    cx.controls,
                );
                if let Some(text) = typed.filter(|text| *text != value) {
                    let mut next = titles.clone();
                    let text = text.trim().to_owned();
                    let ok = match row {
                        TablePart::TitleText(_) => {
                            next[i].title = text.clone();
                            caption_styles::title_text_is_valid(&text)
                        }
                        TablePart::TitleArtist(_) => {
                            next[i].artist = (!text.is_empty()).then_some(text.clone());
                            text.is_empty() || caption_styles::title_text_is_valid(&text)
                        }
                        _ => {
                            let handle = if text.is_empty() || text.starts_with('@') {
                                text.clone()
                            } else {
                                format!("@{text}")
                            };
                            next[i].handle = (!handle.is_empty()).then_some(handle.clone());
                            handle.is_empty() || caption_styles::handle_is_valid(&handle)
                        }
                    };
                    if ok {
                        cx.edits.push(ScenePlanEdit::SetTitles(next));
                    }
                }
                if self.arm_removal(2, i, remove, true) {
                    let mut next = titles.clone();
                    next.remove(i);
                    cx.edits.push(ScenePlanEdit::SetTitles(next));
                }
            }
            TablePart::TitleTimes(i) => {
                let Some(card) = titles.get(i) else { return };
                let first = table_slot::TITLE + i as u32 * table_slot::STRIDE;
                let half = ((rect.width - 6.0) * 0.5).floor();
                for (k, (field, seconds, word)) in [
                    (CardField::TitleStart(i as u8), card.start_seconds, "From"),
                    (CardField::TitleEnd(i as u8), card.end_seconds, "Until"),
                ]
                .into_iter()
                .enumerate()
                {
                    let place = UiRect::new(rect.x + k as f32 * (half + 6.0), rect.y, half, rect.height);
                    let label_width = widgets::measure(font, word, metric::UI_FONT_CAPTION) + 8.0;
                    widgets::draw_text(
                        d,
                        font,
                        word,
                        place.x,
                        place.y + (place.height - metric::UI_FONT_CAPTION) * 0.5,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                    let boxed = UiRect::new(place.x + label_width, place.y, (place.width - label_width).max(0.0), place.height);
                    let shown = widgets::format_timestamp(seconds);
                    let (typed, _) = self.text_row(
                        d,
                        font,
                        boxed,
                        field,
                        first + 4 + k as u32,
                        &shown,
                        "",
                        TextRules::ascii_query(16),
                        None,
                        enabled,
                        cx.controls,
                    );
                    if let Some(at) = typed.as_deref().and_then(musializer_core::ui::lyric_lane_edit::parse_cue_timestamp) {
                        let mut next = titles.clone();
                        if k == 0 {
                            next[i].start_seconds = at;
                        } else {
                            next[i].end_seconds = at;
                        }
                        if caption_styles::titles_are_valid(&next, track.duration_seconds, styles) {
                            cx.edits.push(ScenePlanEdit::SetTitles(next));
                        } else {
                            self.look_column.cards.refused = Some(field.name());
                        }
                    }
                }
            }
            TablePart::TitleStyle(i) => {
                let Some(card) = titles.get(i) else { return };
                let first = table_slot::TITLE + i as u32 * table_slot::STRIDE + 7;
                let names: Vec<Option<caption_styles::StyleName>> =
                    std::iter::once(None).chain(styles.iter().map(|entry| Some(entry.name))).collect();
                for (k, (name, chip)) in names.iter().zip(&cx.place.chips).enumerate() {
                    let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + k as u32);
                    let label = name.map_or_else(|| "Card".to_owned(), |name| name.to_string());
                    let state = self.card_chip(d, font, id, *chip, &label, card.style == *name, enabled);
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        *chip,
                        if name.is_none() {
                            "The card's own look: centred over a soft band"
                        } else {
                            "This style written over the card's look (a lower third: bottom left with a plate)"
                        },
                    );
                    cx.controls.push(format!("title-{i}-style-{label}@{}", rect_text(*chip)));
                    if state.clicked && card.style != *name {
                        let mut next = titles.clone();
                        next[i].style = *name;
                        cx.edits.push(ScenePlanEdit::SetTitles(next));
                    }
                }
            }
            TablePart::AddTitle => {
                let at = cx.time_seconds.max(0.0);
                let duration = track.duration_seconds;
                let after = titles
                    .iter()
                    .map(|card| card.start_seconds)
                    .filter(|start| *start > at)
                    .fold(duration, f64::min);
                let end = (at + caption_styles::DEFAULT_TITLE_SECONDS).min(after).min(duration);
                let blocked = titles.iter().any(|card| card.covers(at))
                    || end - at < caption_styles::MIN_TITLE_SECONDS
                    || titles.len() >= caption_styles::TITLE_CAPACITY;
                let label = format!("+ Add a title card at {}", widgets::format_timestamp(at));
                let width = (widgets::measure(font, &label, metric::UI_FONT_CAPTION) + 24.0).ceil();
                let button = UiRect::new(rect.x, rect.y, width.min(rect.width), rect.height);
                let id = widgets::widget_id(widgets::id::LOOK_CARDS, table_slot::ADD_TITLE);
                if enabled && !blocked {
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        button,
                        &label,
                        false,
                        widgets::ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        button,
                        "A card with the project's title and its artist, for five seconds from the playhead: type over any of it",
                    );
                    cx.controls.push(format!("add-title@{}", rect_text(button)));
                    if state.clicked {
                        let title = track
                            .project_metadata
                            .as_ref()
                            .map(|metadata| metadata.title.clone())
                            .filter(|title| caption_styles::title_text_is_valid(title))
                            .unwrap_or_else(|| track.display_name().to_string());
                        let mut next = titles.clone();
                        let place = next.iter().position(|card| card.start_seconds > at).unwrap_or(next.len());
                        next.insert(
                            place,
                            TitleCard {
                                start_seconds: at,
                                end_seconds: end,
                                title,
                                artist: None,
                                handle: None,
                                style: None,
                            },
                        );
                        cx.edits.push(ScenePlanEdit::SetTitles(next));
                    }
                } else {
                    self.widgets.disabled_button(d, font, button, &label, Some(metric::UI_FONT_CAPTION));
                    let state = self.widgets.button(d, id, button);
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        button,
                        if enabled {
                            "A card is already on screen at the playhead, or there is no room before the next one: move the playhead"
                        } else {
                            FOLLOWING
                        },
                    );
                    cx.controls.push(format!("add-title@disabled:{}", rect_text(button)));
                }
            }
        }
    }
}

/// Words wrapped into `rect`, as many lines as fit.
pub(super) fn draw_wrapped(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    rect: UiRect,
    text: &str,
    tint: raylib::prelude::Color,
) {
    let measure = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
    for (line, words) in wrap(text, rect.width, &measure).iter().enumerate() {
        if (line as f32 + 1.0) * look_cards::LINE_HEIGHT > rect.height + 0.5 {
            break;
        }
        widgets::draw_text(
            d,
            font,
            words,
            rect.x,
            rect.y + line as f32 * look_cards::LINE_HEIGHT + 1.0,
            metric::UI_FONT_CAPTION,
            tint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_name_skips_the_taken_ones() {
        let taken = vec!["style-1".to_owned(), "style-2".to_owned()];
        assert_eq!(fresh_name("style", &taken), "style-3");
        assert_eq!(fresh_name("voice", &[]), "voice-1");
    }

    #[test]
    fn a_colour_crosses_between_rgb_and_rgba() {
        assert_eq!(packed_rgba(0x123456), 0x123456ff);
        assert_eq!(packed_rgb(Some(0x123456ff)), Some(0x123456));
        assert_eq!(packed_rgb(None), None);
    }
}
