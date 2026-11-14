//! Look's look cards (D, MV9-l and MV3-a): the selected section's arrival,
//! glide, grade and caption style, and the whole video's intro, outro and
//! finish, edited where Look already edits how the picture looks.
//!
//! Every write leaves as a [`ScenePlanEdit`] across the plan boundary the
//! Pixel look group uses, so the model validates it, the project is dirtied
//! and autosaved like any plan edit, and the lane's marks follow on the next
//! frame because they are drawn from the same plan. Geometry and the slider
//! scales are `core::ui::tune_groups::look_cards`'.
use super::*;
use crate::ui::panels::scene_timeline::ScenePlanEdit;
use musializer_core::project::caption_styles::StyleName;
use musializer_core::project::look::{grade_bounds, Grade, Palette};
use musializer_core::project::scene_switch::SceneSwitchCue;
use musializer_core::project::transition::{Transition, TransitionKind, DEFAULT_SECONDS};
use musializer_core::ui::tune_groups::look_cards::{self, Block, Placed};

/// Which card the block shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CardScope {
    #[default]
    Section,
    Video,
}

impl CardScope {
    fn name(self) -> &'static str {
        match self {
            Self::Section => "section",
            Self::Video => "video",
        }
    }
}

/// A value that can be typed into, by the value chip at its row's right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardField {
    Length,
    Glide,
    Map,
    ShiftFrom,
    ShiftBy,
    ShiftWidth,
}

impl CardField {
    fn name(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Glide => "glide",
            Self::Map => "map",
            Self::ShiftFrom => "shift-from",
            Self::ShiftBy => "shift-by",
            Self::ShiftWidth => "shift-width",
        }
    }
}

/// Play the cut, while it plays: where it ends, whether to pause there, and
/// whether playback has been seen to start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CutPreview {
    from: f64,
    until: f64,
    pause: bool,
    started: bool,
    frames: u32,
}

/// What `--ui-probe look-action=` asks of a focused card field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardProbeAction {
    Draft,
    Commit,
    Cancel,
}

/// The cards' state across frames, on [`LookColumn`].
#[derive(Clone, Debug, Default)]
pub(crate) struct CardState {
    pub(crate) scope: CardScope,
    typing: Option<(CardField, TextField)>,
    cut: Option<CutPreview>,
    /// The followed treatment's file name, set every frame by the
    /// composition root; `None` while nothing is followed.
    pub(crate) followed: Option<String>,
    probe_text: Option<(Option<String>, CardProbeAction)>,
    report: String,
    /// The section the last frame's card edited, so a new selection takes the
    /// card back to the section scope and drops a half-typed value.
    shown: Option<u64>,
}

/// Slots in [`widgets::id::LOOK_CARDS`]. Sliders stride by
/// [`widgets::SLIDER_ROW_IDS`].
mod slot {
    pub const SCOPE: u32 = 0;
    pub const PLAY: u32 = 10;
    pub const INVITE: u32 = 11;
    pub const STOP_FOLLOW: u32 = 12;
    pub const DONE: u32 = 13;
    pub const KIND: u32 = 20;
    pub const LENGTH: u32 = 40;
    pub const GLIDE: u32 = 44;
    pub const PALETTE: u32 = 60;
    pub const MAP: u32 = 70;
    pub const SHIFT_FROM: u32 = 74;
    pub const SHIFT_BY: u32 = 78;
    pub const SHIFT_WIDTH: u32 = 82;
    pub const STYLE: u32 = 100;
    /// Hover targets for rows drawn disabled, so their reason can be read.
    pub const DISABLED: u32 = 150;
}

/// One drawn part of a card, in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Scope,
    Follow,
    StopFollow,
    Empty,
    Invite,
    InviteButton,
    Title,
    Sub,
    ArrivesLabel,
    Kinds,
    Length,
    Glide,
    GradeLabel,
    Palettes,
    Map,
    ShiftFrom,
    ShiftBy,
    ShiftWidth,
    StyleLabel,
    Styles,
    StyleNone,
    Status,
    VideoNotBuilt,
    Space,
}

/// The cards this frame: what to draw, in order, before any of it is placed.
pub(super) struct CardsPlan {
    parts: Vec<(Part, Block)>,
}

/// The kinds a section can arrive by, in the order the grid shows them:
/// `cut` first, then the vocabulary's own order.
const KINDS: [TransitionKind; 14] = TransitionKind::ALL;

fn kind_label(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::Cut => "Cut",
        TransitionKind::Fade => "Fade",
        TransitionKind::Flash => "Flash",
        TransitionKind::Wipe => "Wipe",
        TransitionKind::Iris => "Iris",
        TransitionKind::Zoom => "Zoom",
        TransitionKind::Slices => "Slices",
        TransitionKind::Glitch => "Glitch",
        TransitionKind::Burn => "Burn",
        TransitionKind::Pixel => "Pixel",
        TransitionKind::Dip => "Dip",
        TransitionKind::Blinds => "Blinds",
        TransitionKind::Ripple => "Ripple",
        TransitionKind::Portal => "Portal",
    }
}

fn palette_label(palette: Palette) -> &'static str {
    match palette {
        Palette::NightDoor => "Night door",
        Palette::Ember => "Ember",
        Palette::Ice => "Ice",
        Palette::Mono => "Mono",
        Palette::Sepia => "Sepia",
        Palette::Dusk => "Dusk",
    }
}

/// What the grade row shows selected: none, a named palette, or colours a
/// treatment listed (shown, never offered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GradeChoice {
    None,
    Named(Palette),
    Listed,
}

fn grade_choice(grade: Option<Grade>) -> GradeChoice {
    match grade {
        None => GradeChoice::None,
        Some(grade) => grade
            .named_palette()
            .map_or(GradeChoice::Listed, GradeChoice::Named),
    }
}

/// The selected section, its position, and the one before it.
fn target<'a>(
    cues: &'a [SceneSwitchCue],
    selected: u64,
) -> Option<(usize, &'a SceneSwitchCue, Option<&'a SceneSwitchCue>)> {
    let index = cues.iter().position(|cue| cue.id == selected)?;
    Some((index, &cues[index], index.checked_sub(1).map(|i| &cues[i])))
}

/// Why a section's glide does nothing, when it does nothing: a glide eases
/// from the section before when it is the same scene (MV9-c).
fn glide_refusal(
    index: usize,
    cue: &SceneSwitchCue,
    previous: Option<&SceneSwitchCue>,
) -> Option<String> {
    match previous {
        None => Some("The first section has nothing to glide from.".to_owned()),
        Some(previous) if previous.scene_index != cue.scene_index => {
            let name = SceneId::from_index(previous.scene_index as usize)
                .map_or("another scene", SceneId::display_name);
            Some(format!(
                "A glide eases in from the section before when it is the same scene; section {} is {name}.",
                index
            ))
        }
        Some(_) => None,
    }
}

fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && measure(&candidate) > width {
            lines.push(std::mem::take(&mut current));
            current = word.to_owned();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn rect_text(r: UiRect) -> String {
    format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height)
}

/// The scope switch and the gap under it, above the card's frame.
const SCOPE_BLOCK: f32 = look_cards::BUTTON_HEIGHT + 8.0;

impl Shell {
    /// `--ui-probe look-text=…,look-action=…`: delivered to a focused card
    /// field on the next frame, refused (and said so) with none focused.
    pub(crate) fn probe_look_text(&mut self, text: Option<String>, action: CardProbeAction) {
        if self.look_column.cards.typing.is_some() {
            self.look_column.cards.probe_text = Some((text, action));
        } else {
            self.look_column
                .cards
                .report
                .push_str(" probe=refused-no-focus");
        }
    }

    /// The followed treatment's file name, or `None`; set every frame by the
    /// composition root, which owns the follow (WX-3).
    pub(crate) fn set_followed_treatment(&mut self, name: Option<String>) {
        if name.is_some() {
            self.look_column.cards.typing = None;
        }
        self.look_column.cards.followed = name;
    }

    /// `--ui-probe section=N`: selects plan section N, as a click on its
    /// block does. Out of range selects nothing.
    pub(crate) fn probe_select_section(&mut self, workspace: &Workspace, number: usize) {
        self.scene_lane.selected_id = workspace
            .current()
            .and_then(|track| track.scene_switches.cues().get(number - 1))
            .map_or(0, |cue| cue.id);
    }

    /// `--ui-probe look=video|section`: which card the block shows.
    pub(crate) fn probe_look_scope(&mut self, scope: CardScope) {
        self.look_column.cards.scope = scope;
    }

    /// Whether a card value is being typed: part of
    /// [`Shell::tune_value_typing`], so a digit typed into a length does not
    /// select a scene (UX0-A06, D7).
    pub(crate) fn card_typing(&self) -> bool {
        self.look_column.cards.typing.is_some()
    }

    /// The `look cards:` report line.
    pub(crate) fn describe_look_cards(&self) -> String {
        if self.column_view() != super::super::super::shell::ColumnView::Look {
            return String::new();
        }
        self.look_column.cards.report.clone()
    }

    /// Plays the cut while it plays, and pauses where it ends when playback
    /// was paused before the press. Run every frame Look is drawn.
    fn tend_cut(&mut self, input: &ShellInput<'_>, commands: &mut Vec<ShellCommand>) {
        let Some(mut cut) = self.look_column.cards.cut else {
            return;
        };
        cut.frames += 1;
        if input.playing {
            cut.started = true;
            if input.time_seconds >= cut.until {
                if cut.pause {
                    commands.push(ShellCommand::TogglePlay);
                }
                self.look_column.cards.cut = None;
                return;
            }
        } else if cut.started || cut.frames > 30 {
            // Paused by hand, or playback never started: nothing to finish.
            self.look_column.cards.cut = None;
            return;
        }
        self.look_column.cards.cut = Some(cut);
    }

    /// What the cards will draw this frame. Built before the list is laid
    /// out, so the Look list reserves exactly the height they take.
    pub(super) fn plan_look_cards(&mut self, input: &ShellInput<'_>) -> CardsPlan {
        let mut parts = vec![
            (Part::Scope, Block::Buttons),
            (Part::Space, Block::Gap(8.0)),
        ];
        let Some(track) = input.workspace.current() else {
            return CardsPlan { parts: Vec::new() };
        };
        let selected = self.scene_lane.selected_id;
        if self.look_column.cards.shown != Some(selected) {
            self.look_column.cards.shown = Some(selected);
            self.look_column.cards.typing = None;
            if selected != 0 {
                self.look_column.cards.scope = CardScope::Section;
            }
        }
        let following = self.look_column.cards.followed.is_some();
        if following {
            parts.push((Part::Follow, Block::Lines(3)));
            parts.push((Part::StopFollow, Block::Buttons));
            parts.push((Part::Space, Block::Gap(look_cards::GROUP_GAP)));
        }
        if self.look_column.cards.scope == CardScope::Video {
            parts.push((Part::VideoNotBuilt, Block::Lines(3)));
            return CardsPlan { parts };
        }
        let cues = track.scene_switches.cues();
        if cues.is_empty() {
            parts.push((Part::Empty, Block::Lines(3)));
            return CardsPlan { parts };
        }
        let Some((_, cue, _)) = target(cues, selected) else {
            parts.push((Part::Invite, Block::Lines(2)));
            parts.push((Part::InviteButton, Block::Buttons));
            return CardsPlan { parts };
        };
        parts.extend([
            (Part::Title, Block::Buttons),
            (Part::Sub, Block::Line),
            (Part::Space, Block::Gap(look_cards::GROUP_GAP)),
            (Part::ArrivesLabel, Block::Label),
            (Part::Kinds, Block::Grid(KINDS.len())),
            (Part::Space, Block::Gap(6.0)),
            (Part::Length, Block::Slider),
            (Part::Glide, Block::Slider),
            (Part::Space, Block::Gap(look_cards::GROUP_GAP)),
            (Part::GradeLabel, Block::Label),
        ]);
        let listed = grade_choice(cue.grade) == GradeChoice::Listed;
        parts.push((
            Part::Palettes,
            Block::Grid(1 + Palette::ALL.len() + usize::from(listed)),
        ));
        if cue.grade.is_some() {
            parts.extend([
                (Part::Space, Block::Gap(6.0)),
                (Part::Map, Block::Slider),
                (Part::ShiftBy, Block::Slider),
                (Part::ShiftFrom, Block::Slider),
                (Part::ShiftWidth, Block::Slider),
            ]);
        }
        parts.push((Part::Space, Block::Gap(look_cards::GROUP_GAP)));
        parts.push((Part::StyleLabel, Block::Label));
        let styles = track.caption_style.styles.len();
        if styles == 0 {
            parts.push((Part::StyleNone, Block::Lines(2)));
        } else {
            parts.push((Part::Styles, Block::Grid(1 + styles)));
        }
        parts.push((Part::Space, Block::Gap(8.0)));
        parts.push((Part::Status, Block::Line));
        CardsPlan { parts }
    }

    /// The block's height in a list `width` wide: the scope switch, then the
    /// card's frame around the rest.
    pub(super) fn look_cards_height(plan: &CardsPlan, width: f32) -> f32 {
        if plan.parts.is_empty() {
            return 0.0;
        }
        let inner = (width - look_cards::CARD_PADDING * 2.0).max(0.0);
        let blocks: Vec<Block> = plan.parts[2..].iter().map(|(_, block)| *block).collect();
        let (_, content) = look_cards::layout(0.0, 0.0, inner, &blocks);
        SCOPE_BLOCK + look_cards::card_height(content)
    }

    fn place_cards(plan: &CardsPlan, area: UiRect) -> (Vec<(Part, Placed)>, UiRect) {
        let mut placed = Vec::with_capacity(plan.parts.len());
        let head: Vec<Block> = plan.parts[..2].iter().map(|(_, block)| *block).collect();
        let (top, _) = look_cards::layout(area.x, area.y, area.width, &head);
        placed.extend(plan.parts[..2].iter().map(|(part, _)| *part).zip(top));
        let frame = UiRect::new(
            area.x,
            area.y + SCOPE_BLOCK,
            area.width,
            (area.height - SCOPE_BLOCK).max(0.0),
        );
        let blocks: Vec<Block> = plan.parts[2..].iter().map(|(_, block)| *block).collect();
        let (body, _) = look_cards::layout(
            frame.x + look_cards::CARD_PADDING,
            frame.y + look_cards::CARD_PADDING,
            (frame.width - look_cards::CARD_PADDING * 2.0).max(0.0),
            &blocks,
        );
        placed.extend(plan.parts[2..].iter().map(|(part, _)| *part).zip(body));
        (placed, frame)
    }

    /// One chip of a grid: the selection look when chosen, a hover box when
    /// pointed at, muted words otherwise; drawn disabled (and never clicked)
    /// when `enabled` is false. The hit target exists either way, so a
    /// disabled chip can say why in its tooltip.
    #[allow(
        clippy::too_many_arguments,
        reason = "a button's arguments plus its two states"
    )]
    fn card_chip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        id: u64,
        rect: UiRect,
        label: &str,
        selected: bool,
        enabled: bool,
    ) -> widgets::ButtonState {
        let state = self.widgets.button(d, id, rect);
        let hovered = enabled && state.hovered;
        let (fill, edge) = if selected {
            (color::ui_selected(), color::ui_selected_edge())
        } else if hovered {
            (color::ui_hover(), color::ui_control_edge())
        } else {
            (color::ui_control(), color::ui_control_edge())
        };
        widgets::control_box(d, rect, fill, edge, 1.0);
        let ink = if !enabled {
            color::ui_disabled()
        } else if selected || hovered {
            color::ui_ink()
        } else {
            color::ui_muted()
        };
        widgets::text_label(d, font, rect, label, metric::UI_FONT_CAPTION, ink);
        widgets::ButtonState {
            clicked: enabled && state.clicked,
            ..state
        }
    }

    /// A slider row that cannot be moved: its label, a flat bar and its value
    /// in the disabled tone, and a hover target for the reason.
    #[allow(clippy::too_many_arguments, reason = "a row's content and its reason")]
    fn disabled_card_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        index: u32,
        row: UiRect,
        label: &str,
        value: &str,
        reason: &str,
        label_width: f32,
    ) -> UiRect {
        let layout = widgets::slider_row_layout(row, label_width, false);
        let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::DISABLED + index);
        let state = self.widgets.button(d, id, layout.row);
        let y = layout.row.y + (layout.row.height - metric::UI_FONT_CAPTION) * 0.5;
        widgets::draw_text(
            d,
            font,
            label,
            layout.label.x + 10.0,
            y,
            metric::UI_FONT_CAPTION,
            color::ui_disabled(),
        );
        widgets::fill(d, layout.bar, color::ui_rule());
        let width = widgets::measure_tabular(font, value, metric::UI_FONT_CAPTION);
        widgets::draw_text_tabular(
            d,
            font,
            value,
            layout.value.x + layout.value.width - width,
            y,
            metric::UI_FONT_CAPTION,
            color::ui_disabled(),
        );
        self.widgets.hint(d, state, id, layout.row, reason);
        layout.row
    }

    /// A micro label with a trailing value in ink, right-aligned.
    fn card_label(
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        rect: UiRect,
        label: &str,
        value: &str,
    ) {
        widgets::section_head(d, font, rect, &[widgets::MicroRun::muted(label)]);
        if value.is_empty() {
            return;
        }
        let runs = [widgets::MicroRun::ink(value)];
        let width = widgets::micro_width(font, &runs);
        let x = rect.x + rect.width - width;
        // The rule runs under the value; clear it there so the value reads.
        widgets::fill(
            d,
            UiRect::new(x - 6.0, rect.y, width + 6.0, rect.height),
            color::ui_raised(),
        );
        widgets::draw_micro(
            d,
            font,
            x,
            rect.y + (rect.height - metric::UI_FONT_MICRO) * 0.5,
            &runs,
        );
    }

    /// One slider row of a card, with a value chip that opens typed entry.
    /// Returns the asked-for normalized value while dragged, the committed
    /// typed text, and the row's rects for the report.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row: its id, place, content and field"
    )]
    fn card_slider(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        first: u32,
        row: UiRect,
        spec: &widgets::SliderRow<'_>,
        field: CardField,
        enabled: bool,
    ) -> (Option<f32>, Option<String>, String) {
        let result = self
            .widgets
            .slider_row(d, font, widgets::id::LOOK_CARDS, first, row, spec);
        let mut typed = None;
        if !enabled {
            return (None, None, String::new());
        }
        if result.value.clicked {
            let mut text = TextField::new(TextRules::ascii_query(8));
            text.bind(
                spec.value_text
                    .trim_end_matches(" s")
                    .trim_end_matches('°')
                    .trim_end_matches('%'),
            );
            text.set_focused(true);
            self.look_column.cards.typing = Some((field, text));
        }
        if let Some((typing, text)) = self.look_column.cards.typing.as_mut() {
            if *typing == field {
                let probe = self.look_column.cards.probe_text.take();
                if let Some((Some(value), _)) = &probe {
                    text.bind(value);
                    text.set_focused(true);
                }
                let commit = d.is_key_pressed(KeyboardKey::KEY_ENTER)
                    || d.is_key_pressed(KeyboardKey::KEY_KP_ENTER)
                    || matches!(probe, Some((_, CardProbeAction::Commit)));
                let cancel = d.is_key_pressed(KeyboardKey::KEY_ESCAPE)
                    || matches!(probe, Some((_, CardProbeAction::Cancel)));
                widgets::fill(d, result.layout.value, color::ui_surface());
                text.draw_with_face(
                    d,
                    AuthoredText::from_ui(font),
                    result.layout.value,
                    metric::UI_FONT_VALUE,
                    "",
                    true,
                );
                if commit {
                    typed = Some(text.edit.text().to_owned());
                }
                if commit || cancel || !text.is_focused() {
                    self.look_column.cards.typing = None;
                }
            }
        }
        let report = format!(
            "{}@{};{}-value@{}",
            field.name(),
            rect_text(result.layout.bar),
            field.name(),
            rect_text(result.layout.value)
        );
        (result.drag, typed, report)
    }

    /// The cards, drawn into `area` (the rect the Look list reserved).
    #[allow(
        clippy::too_many_lines,
        reason = "one card's parts in drawing order; each arm is short"
    )]
    pub(super) fn look_cards(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        plan: &CardsPlan,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        self.tend_cut(input, commands);
        let Some(track) = input.workspace.current() else {
            self.look_column.cards.report.clear();
            return;
        };
        if plan.parts.is_empty() {
            self.look_column.cards.report.clear();
            return;
        }
        let font = input.fonts.ui();
        let (placed, frame) = Self::place_cards(plan, area);
        widgets::rounded_box(d, frame, 4.0, color::ui_raised(), color::ui_rule(), 1.0);
        let followed = self.look_column.cards.followed.clone();
        let enabled = followed.is_none();
        let cues = track.scene_switches.cues();
        let selected = target(cues, self.scene_lane.selected_id);
        let label_width = look_label_width(frame.width - look_cards::CARD_PADDING * 2.0).min(96.0);
        let mut controls: Vec<String> = Vec::new();
        let mut edits: Vec<ScenePlanEdit> = Vec::new();
        let measure = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
        let draw_lines = |d: &mut RaylibDrawHandle<'_>, rect: UiRect, text: &str, tint| {
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
        };

        for (part, place) in &placed {
            let rect = place.rect;
            match part {
                Part::Scope => {
                    let picked = self.widgets.segmented(
                        d,
                        font,
                        widgets::id::LOOK_CARDS,
                        slot::SCOPE,
                        rect,
                        &["This section", "Whole video"],
                        Some(usize::from(self.look_column.cards.scope == CardScope::Video)),
                        widgets::SegmentedFit::Fill,
                    );
                    for (i, item) in picked.layout.items.iter().enumerate() {
                        controls.push(format!("scope-{i}@{}", rect_text(*item)));
                    }
                    if let Some(i) = picked.clicked {
                        self.look_column.cards.scope =
                            if i == 0 { CardScope::Section } else { CardScope::Video };
                        self.look_column.cards.typing = None;
                    }
                }
                Part::Follow => {
                    let name = followed.as_deref().unwrap_or("a treatment");
                    draw_lines(
                        d,
                        rect,
                        &format!(
                            "Following {name}: the file decides how this video looks, and its next save would undo an edit made here. Stop following to edit in the window."
                        ),
                        color::ui_warning(),
                    );
                }
                Part::StopFollow => {
                    let button = UiRect::new(rect.x, rect.y, 128.0f32.min(rect.width), rect.height);
                    let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::STOP_FOLLOW);
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        button,
                        "Stop following",
                        false,
                        widgets::ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    controls.push(format!("stop-follow@{}", rect_text(button)));
                    if state.clicked {
                        commands.push(ShellCommand::StopFollowingTreatment);
                    }
                }
                Part::Empty => draw_lines(
                    d,
                    rect,
                    "This video has no sections yet. Split here in the plan lane adds one at the playhead; the whole video's look is under Whole video.",
                    color::ui_muted(),
                ),
                Part::VideoNotBuilt => draw_lines(
                    d,
                    rect,
                    "Not built yet: the intro, the outro and the finish come from a treatment until this card lands.",
                    color::ui_muted(),
                ),
                Part::Invite => draw_lines(
                    d,
                    rect,
                    "Click a section in the plan lane to shape how it arrives, how it glides and what palette it wears.",
                    color::ui_muted(),
                ),
                Part::InviteButton => {
                    let playing = track
                        .scene_switches
                        .cues()
                        .iter()
                        .position(|cue| {
                            cue.start_seconds <= input.time_seconds
                                && input.time_seconds < cue.end_seconds
                        })
                        .unwrap_or(cues.len() - 1);
                    let label = format!("Section {} (playing now)", playing + 1);
                    let width = (measure(&label) + 24.0).min(rect.width);
                    let button = UiRect::new(rect.x, rect.y, width, rect.height);
                    let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::INVITE);
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
                    controls.push(format!("invite@{}", rect_text(button)));
                    if state.clicked {
                        self.scene_lane.selected_id = cues[playing].id;
                    }
                }
                Part::Title => {
                    let Some((index, cue, _)) = selected else { continue };
                    widgets::draw_text(
                        d,
                        font,
                        &format!("Section {} of {}", index + 1, cues.len()),
                        rect.x,
                        rect.y + (rect.height - metric::UI_FONT_LABEL) * 0.5,
                        metric::UI_FONT_LABEL,
                        color::ui_ink(),
                    );
                    let label = if index == 0 { "Play the intro" } else { "Play the cut" };
                    let width = (measure(label) + 24.0).ceil();
                    let button = UiRect::new(rect.x + rect.width - width, rect.y, width, rect.height);
                    let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::PLAY);
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id,
                        button,
                        label,
                        self.look_column.cards.cut.is_some(),
                        widgets::ButtonStyle::Strong,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(
                        d,
                        state,
                        id,
                        button,
                        "Plays from just before this section's boundary through how it arrives, then pauses where it started paused",
                    );
                    controls.push(format!("play@{}", rect_text(button)));
                    // Done lets the selection go, which folds the card back to
                    // its invitation and gives the scene's tuning the column.
                    let done_width = (measure("Done") + 20.0).ceil();
                    let done = UiRect::new(button.x - 6.0 - done_width, rect.y, done_width, rect.height);
                    let done_id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::DONE);
                    let done_state = self.widgets.text_button(
                        d,
                        font,
                        done_id,
                        done,
                        "Done",
                        false,
                        widgets::ButtonStyle::Ghost,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(d, done_state, done_id, done, "Let the section go: the card folds away");
                    controls.push(format!("done@{}", rect_text(done)));
                    if done_state.clicked {
                        self.scene_lane.selected_id = 0;
                    }
                    if state.clicked {
                        let room = cue.end_seconds - cue.start_seconds;
                        let seconds = if cue.transition.is_cut() {
                            0.0
                        } else {
                            musializer_core::project::transition::fitted_seconds(cue.transition, room)
                        };
                        let (from, until) = look_cards::cut_span(
                            cue.start_seconds,
                            seconds,
                            cue.transition.kind == TransitionKind::Dip,
                            index == 0,
                        );
                        let until = until.min(input.duration_seconds);
                        commands.push(ShellCommand::Seek(from));
                        if !input.playing {
                            commands.push(ShellCommand::TogglePlay);
                        }
                        self.look_column.cards.cut = Some(CutPreview {
                            from,
                            until,
                            pause: !input.playing,
                            started: false,
                            frames: 0,
                        });
                    }
                }
                Part::Sub => {
                    let Some((_, cue, _)) = selected else { continue };
                    let scene = SceneId::from_index(cue.scene_index as usize)
                        .map_or("Unknown scene", SceneId::display_name);
                    let text = format!(
                        "{scene} \u{00b7} {} to {} \u{00b7} {:.1} s",
                        widgets::format_timestamp(cue.start_seconds),
                        widgets::format_timestamp(cue.end_seconds),
                        cue.end_seconds - cue.start_seconds
                    );
                    widgets::draw_text_faded(
                        d,
                        font,
                        &ascii_fallback(font.all_loaded(), &text),
                        rect.x,
                        rect.y,
                        rect.width,
                        16.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
                Part::ArrivesLabel => {
                    let Some((index, cue, _)) = selected else { continue };
                    let value = if cue.transition.is_cut() {
                        if index == 0 { "starts on the picture".to_owned() } else { "a cut".to_owned() }
                    } else {
                        format!("{} \u{00b7} {:.2} s", kind_label(cue.transition.kind), cue.transition.seconds)
                    };
                    Self::card_label(
                        d,
                        font,
                        rect,
                        if index == 0 { "Opens out of black" } else { "Arrives by" },
                        &ascii_fallback(font.all_loaded(), &value),
                    );
                }
                Part::Kinds => {
                    let Some((_, cue, _)) = selected else { continue };
                    for (i, (kind, chip)) in KINDS.iter().zip(&place.chips).enumerate() {
                        let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::KIND + i as u32);
                        let state = self.card_chip(
                            d,
                            font,
                            id,
                            *chip,
                            kind_label(*kind),
                            cue.transition.kind == *kind,
                            enabled,
                        );
                        self.widgets.hint(d, state, id, *chip, kind.look());
                        controls.push(format!("kind-{}@{}", kind.name(), rect_text(*chip)));
                        if state.clicked && cue.transition.kind != *kind {
                            let seconds = if cue.transition.is_cut() {
                                DEFAULT_SECONDS
                            } else {
                                cue.transition.seconds
                            };
                            edits.push(ScenePlanEdit::SetTransition {
                                cue_id: cue.id,
                                transition: Transition::new(*kind, seconds),
                            });
                        }
                    }
                }
                Part::Length => {
                    let Some((_, cue, _)) = selected else { continue };
                    if cue.transition.is_cut() || !enabled {
                        let reason = if enabled {
                            "A cut has no length: pick how the section arrives first."
                        } else {
                            "Following a treatment: stop following to edit here."
                        };
                        let value = if cue.transition.is_cut() {
                            "\u{2014}".to_owned()
                        } else {
                            format!("{:.2} s", cue.transition.seconds)
                        };
                        let row = self.disabled_card_row(d, font, 0, rect, "Length", &ascii_fallback(font.all_loaded(), &value), reason, label_width);
                        controls.push(format!("length@disabled:{}", rect_text(row)));
                        continue;
                    }
                    let value = format!("{:.2} s", cue.transition.seconds);
                    let (drag, typed, report) = self.card_slider(
                        d,
                        font,
                        slot::LENGTH,
                        rect,
                        &widgets::SliderRow {
                            label: "Length",
                            value_text: &value,
                            normalized: look_cards::transition_unit(cue.transition.seconds),
                            default: Some(look_cards::transition_unit(DEFAULT_SECONDS)),
                            changed: (cue.transition.seconds - DEFAULT_SECONDS).abs() > 0.004,
                            route: None,
                            label_width,
                        },
                        CardField::Length,
                        enabled,
                    );
                    controls.push(report);
                    let seconds = drag
                        .map(look_cards::transition_seconds)
                        .or_else(|| {
                            typed.as_deref().and_then(look_cards::parse_seconds).map(|s| {
                                s.clamp(
                                    musializer_core::project::transition::MIN_SECONDS,
                                    musializer_core::project::transition::MAX_SECONDS,
                                )
                            })
                        });
                    if let Some(seconds) = seconds.filter(|s| *s != cue.transition.seconds) {
                        edits.push(ScenePlanEdit::SetTransition {
                            cue_id: cue.id,
                            transition: Transition::new(cue.transition.kind, seconds),
                        });
                    }
                }
                Part::Glide => {
                    let Some((index, cue, previous)) = selected else { continue };
                    let refusal = glide_refusal(index, cue, previous);
                    let value = if cue.glide > 0.0 {
                        format!("{:.2} s", cue.glide)
                    } else {
                        "none".to_owned()
                    };
                    if refusal.is_some() || !enabled {
                        let reason = refusal.unwrap_or_else(|| {
                            "Following a treatment: stop following to edit here.".to_owned()
                        });
                        let row = self.disabled_card_row(d, font, 1, rect, "Glide", &value, &reason, label_width);
                        controls.push(format!("glide@disabled:{}", rect_text(row)));
                        continue;
                    }
                    let (drag, typed, report) = self.card_slider(
                        d,
                        font,
                        slot::GLIDE,
                        rect,
                        &widgets::SliderRow {
                            label: "Glide",
                            value_text: &value,
                            normalized: look_cards::glide_unit(cue.glide),
                            default: Some(0.0),
                            changed: cue.glide > 0.0,
                            route: None,
                            label_width,
                        },
                        CardField::Glide,
                        enabled,
                    );
                    controls.push(report);
                    let seconds = drag.map(look_cards::glide_seconds).or_else(|| {
                        typed
                            .as_deref()
                            .and_then(|text| if text.trim() == "none" { Some(0.0) } else { look_cards::parse_seconds(text) })
                            .map(|s| s.clamp(0.0, musializer_core::project::scene_switch::MAX_GLIDE_SECONDS))
                    });
                    if let Some(seconds) = seconds.filter(|s| *s != cue.glide) {
                        edits.push(ScenePlanEdit::SetGlide { cue_id: cue.id, seconds });
                    }
                }
                Part::GradeLabel => {
                    let Some((_, cue, _)) = selected else { continue };
                    let value = match (grade_choice(cue.grade), cue.grade) {
                        (GradeChoice::None, _) | (_, None) => "the scene's own colour".to_owned(),
                        (GradeChoice::Named(palette), Some(grade)) => {
                            format!("{} \u{00b7} {:.0}%", palette_label(palette), grade.map * 100.0)
                        }
                        (GradeChoice::Listed, Some(grade)) => {
                            format!("listed colours \u{00b7} {:.0}%", grade.map * 100.0)
                        }
                    };
                    Self::card_label(d, font, rect, "Grade", &ascii_fallback(font.all_loaded(), &value));
                }
                Part::Palettes => {
                    let Some((_, cue, _)) = selected else { continue };
                    let shown = grade_choice(cue.grade);
                    let mut choices = vec![GradeChoice::None];
                    choices.extend(Palette::ALL.map(GradeChoice::Named));
                    if shown == GradeChoice::Listed {
                        choices.push(GradeChoice::Listed);
                    }
                    for (i, (choice, chip)) in choices.iter().zip(&place.chips).enumerate() {
                        let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::PALETTE + i as u32);
                        let label = match choice {
                            GradeChoice::None => "None",
                            GradeChoice::Named(palette) => palette_label(*palette),
                            GradeChoice::Listed => "Listed",
                        };
                        let offered = *choice != GradeChoice::Listed;
                        let state = self.card_chip(d, font, id, *chip, label, shown == *choice, enabled && offered);
                        // The palette itself, as a strip along the chip's foot.
                        let stops: Vec<u32> = match choice {
                            GradeChoice::None => Vec::new(),
                            GradeChoice::Named(palette) => palette.stops().to_vec(),
                            GradeChoice::Listed => cue.grade.map(|g| g.stops().to_vec()).unwrap_or_default(),
                        };
                        draw_palette_strip(d, *chip, &stops);
                        let hint = match choice {
                            GradeChoice::None => "No grade: the scene's own colours".to_owned(),
                            GradeChoice::Named(palette) => format!(
                                "Hold the section to {}: the picture's light mapped through its colours",
                                palette_label(*palette)
                            ),
                            GradeChoice::Listed => "Colours the treatment listed; pick a palette to replace them".to_owned(),
                        };
                        self.widgets.hint(d, state, id, *chip, &hint);
                        controls.push(format!(
                            "palette-{}@{}",
                            match choice {
                                GradeChoice::None => "none",
                                GradeChoice::Named(palette) => palette.name(),
                                GradeChoice::Listed => "listed",
                            },
                            rect_text(*chip)
                        ));
                        if state.clicked && shown != *choice {
                            let grade = match choice {
                                GradeChoice::None => None,
                                GradeChoice::Named(palette) => Some(match cue.grade {
                                    Some(grade) => grade.with_palette(*palette),
                                    None => Grade::from_palette(
                                        *palette,
                                        musializer_core::treatment::DEFAULT_GRADE_MAP,
                                    ),
                                }),
                                GradeChoice::Listed => cue.grade,
                            };
                            edits.push(ScenePlanEdit::SetGrade { cue_id: cue.id, grade });
                        }
                    }
                }
                Part::Map | Part::ShiftBy | Part::ShiftFrom | Part::ShiftWidth => {
                    let Some((_, cue, _)) = selected else { continue };
                    let Some(grade) = cue.grade else { continue };
                    let (label, field, first, value, (low, high), default, text) = match part {
                        Part::Map => (
                            "Strength",
                            CardField::Map,
                            slot::MAP,
                            grade.map,
                            grade_bounds::MAP,
                            musializer_core::treatment::DEFAULT_GRADE_MAP,
                            format!("{:.0}%", grade.map * 100.0),
                        ),
                        Part::ShiftBy => (
                            "Hue turn",
                            CardField::ShiftBy,
                            slot::SHIFT_BY,
                            grade.shift_by,
                            grade_bounds::SHIFT_BY,
                            0.0,
                            format!("{:+.0}\u{00b0}", grade.shift_by),
                        ),
                        Part::ShiftFrom => (
                            "Turns hues at",
                            CardField::ShiftFrom,
                            slot::SHIFT_FROM,
                            grade.shift_from,
                            grade_bounds::SHIFT_FROM,
                            125.0,
                            format!("{:.0}\u{00b0}", grade.shift_from),
                        ),
                        _ => (
                            "Turn width",
                            CardField::ShiftWidth,
                            slot::SHIFT_WIDTH,
                            grade.shift_width,
                            grade_bounds::SHIFT_WIDTH,
                            32.0,
                            format!("{:.0}\u{00b0}", grade.shift_width),
                        ),
                    };
                    let text = ascii_fallback(font.all_loaded(), &text);
                    let (drag, typed, report) = self.card_slider(
                        d,
                        font,
                        first,
                        rect,
                        &widgets::SliderRow {
                            label,
                            value_text: &text,
                            normalized: (value - low) / (high - low),
                            default: Some((default - low) / (high - low)),
                            changed: (value - default).abs() > 0.004,
                            route: None,
                            label_width,
                        },
                        field,
                        enabled,
                    );
                    controls.push(report);
                    let asked = drag.map(|unit| low + unit * (high - low)).or_else(|| {
                        typed.as_deref().and_then(|text| {
                            let text = text.trim().trim_end_matches(['%', '\u{00b0}']).trim();
                            let number: f32 = text.parse().ok()?;
                            number.is_finite().then(|| if field == CardField::Map { number / 100.0 } else { number })
                        })
                    });
                    if let Some(asked) = asked {
                        // Whole degrees and whole percent, as the readout prints.
                        let snapped = if field == CardField::Map {
                            (asked * 100.0).round() / 100.0
                        } else {
                            asked.round()
                        }
                        .clamp(low, high);
                        let mut next = grade;
                        match field {
                            CardField::Map => next.map = snapped,
                            CardField::ShiftBy => next.shift_by = snapped,
                            CardField::ShiftFrom => next.shift_from = snapped,
                            _ => next.shift_width = snapped,
                        }
                        if next != grade {
                            edits.push(ScenePlanEdit::SetGrade {
                                cue_id: cue.id,
                                grade: Some(next),
                            });
                        }
                    }
                }
                Part::StyleLabel => {
                    let Some((_, cue, _)) = selected else { continue };
                    let value = cue
                        .caption_style
                        .map_or_else(|| "base".to_owned(), |name| name.to_string());
                    Self::card_label(d, font, rect, "Captions begin in", &value);
                }
                Part::StyleNone => draw_lines(
                    d,
                    rect,
                    "This project has no named caption styles; a treatment's caption_styles adds them.",
                    color::ui_muted(),
                ),
                Part::Styles => {
                    let Some((_, cue, _)) = selected else { continue };
                    let table = &track.caption_style.styles;
                    let names: Vec<Option<StyleName>> = std::iter::once(None)
                        .chain(table.iter().map(|entry| Some(entry.name)))
                        .collect();
                    for (i, (name, chip)) in names.iter().zip(&place.chips).enumerate() {
                        let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::STYLE + i as u32);
                        let label = name.map_or_else(|| "Base".to_owned(), |n| n.to_string());
                        let state = self.card_chip(d, font, id, *chip, &label, cue.caption_style == *name, enabled);
                        let hint = match name {
                            None => "Lines that begin here wear the project's caption style".to_owned(),
                            Some(name) => format!(
                                "Lines that begin here wear {name}: {}",
                                musializer_core::project::caption_styles::find(table, *name)
                                    .map_or_else(String::new, |style| style.describe())
                            ),
                        };
                        self.widgets.hint(d, state, id, *chip, &hint);
                        controls.push(format!("style-{label}@{}", rect_text(*chip)));
                        if state.clicked && cue.caption_style != *name {
                            edits.push(ScenePlanEdit::SetCaptionStyle { cue_id: cue.id, style: *name });
                        }
                    }
                }
                Part::Status => {
                    let (text, tint) = if !track.scene_switches.enabled {
                        (
                            "The plan is off: Auto on in the plan lane plays these.",
                            color::ui_warning(),
                        )
                    } else if followed.is_some() {
                        ("The followed file decides.", color::ui_muted())
                    } else {
                        ("Saved with the project. The lane shows each change.", color::ui_muted())
                    };
                    widgets::draw_text_faded(
                        d,
                        font,
                        text,
                        rect.x,
                        rect.y,
                        rect.width,
                        16.0,
                        metric::UI_FONT_MICRO,
                        tint,
                    );
                }
                Part::Space => {}
            }
        }
        if !enabled {
            edits.clear();
        }
        commands.extend(edits.into_iter().map(ShellCommand::ScenePlan));

        let target_text = selected.map_or_else(
            || "none".to_owned(),
            |(index, cue, _)| {
                format!(
                    "{} id={} kind={} seconds={:.2} glide={:.2} grade={} style={}",
                    index + 1,
                    cue.id,
                    cue.transition.kind.name(),
                    cue.transition.seconds,
                    cue.glide,
                    grade_text(cue.grade),
                    cue.caption_style
                        .map_or_else(|| "base".to_owned(), |name| name.to_string()),
                )
            },
        );
        let cut = self.look_column.cards.cut.map_or_else(
            || "none".to_owned(),
            |cut| {
                format!(
                    "{:.3}..{:.3}{}",
                    cut.from,
                    cut.until,
                    if cut.started { " playing" } else { "" }
                )
            },
        );
        let typing = self
            .look_column
            .cards
            .typing
            .as_ref()
            .map_or("none", |(field, _)| field.name());
        let probe = if self
            .look_column
            .cards
            .report
            .ends_with(" probe=refused-no-focus")
        {
            " probe=refused-no-focus"
        } else {
            ""
        };
        self.look_column.cards.report = format!(
            "scope={} target={target_text} follow={} editable={} cut={cut} typing={typing} area={} frame={} controls={}{probe}",
            self.look_column.cards.scope.name(),
            followed.as_deref().unwrap_or("none"),
            u8::from(enabled),
            rect_text(area),
            rect_text(frame),
            controls.join(";"),
        );
    }
}

/// `palette:map:from,by,width`, or `none`.
fn grade_text(grade: Option<Grade>) -> String {
    match grade {
        None => "none".to_owned(),
        Some(grade) => format!(
            "{}:{:.2}:{:.0},{:.0},{:.0}",
            grade.named_palette().map_or("listed", Palette::name),
            grade.map,
            grade.shift_from,
            grade.shift_by,
            grade.shift_width
        ),
    }
}

/// A palette as a 3 px strip along a chip's foot, inset from its corners.
fn draw_palette_strip(d: &mut RaylibDrawHandle<'_>, chip: UiRect, stops: &[u32]) {
    if stops.len() < 2 || chip.width < 12.0 {
        return;
    }
    let strip = UiRect::new(
        chip.x + 4.0,
        chip.y + chip.height - 5.0,
        chip.width - 8.0,
        3.0,
    );
    let pieces = stops.len() - 1;
    let piece = strip.width / pieces as f32;
    let channel = |value: f32| (value * 255.0).round() as u8;
    for index in 0..pieces {
        let [r0, g0, b0] = musializer_core::project::look::rgb(stops[index]);
        let [r1, g1, b1] = musializer_core::project::look::rgb(stops[index + 1]);
        let left = (strip.x + piece * index as f32).round();
        let right = (strip.x + piece * (index + 1) as f32).round();
        d.draw_rectangle_gradient_h(
            left as i32,
            strip.y as i32,
            (right - left).max(1.0) as i32,
            strip.height as i32,
            raylib::prelude::Color::new(channel(r0), channel(g0), channel(b0), 255),
            raylib::prelude::Color::new(channel(r1), channel(g1), channel(b1), 255),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_name_in_the_grid_and_cut_comes_first() {
        assert_eq!(KINDS[0], TransitionKind::Cut);
        let labels: std::collections::BTreeSet<_> =
            KINDS.iter().map(|kind| kind_label(*kind)).collect();
        assert_eq!(labels.len(), KINDS.len());
    }

    #[test]
    fn a_glide_from_another_scene_says_why_it_does_nothing() {
        let first = SceneSwitchCue {
            id: 1,
            scene_index: 0,
            ..SceneSwitchCue::default()
        };
        let same = SceneSwitchCue {
            id: 2,
            scene_index: 0,
            ..SceneSwitchCue::default()
        };
        let other = SceneSwitchCue {
            id: 3,
            scene_index: 1,
            ..SceneSwitchCue::default()
        };
        assert!(glide_refusal(0, &first, None).is_some());
        assert!(glide_refusal(1, &same, Some(&first)).is_none());
        let why = glide_refusal(2, &other, Some(&same)).unwrap();
        assert!(why.contains("section 2"), "{why}");
    }

    #[test]
    fn the_grade_row_reads_a_listed_palette_as_listed_and_a_named_one_by_name() {
        assert_eq!(grade_choice(None), GradeChoice::None);
        let named = Grade::from_palette(Palette::Ember, 0.5);
        assert_eq!(
            grade_choice(Some(named)),
            GradeChoice::Named(Palette::Ember)
        );
        let mut listed = named;
        listed.stops[0] = 0x123456;
        assert_eq!(grade_choice(Some(listed)), GradeChoice::Listed);
        // Another palette keeps the strength and the hue turn.
        let turned = Grade {
            shift_by: 40.0,
            ..named
        };
        let picked = turned.with_palette(Palette::Ice);
        assert_eq!(picked.named_palette(), Some(Palette::Ice));
        assert_eq!((picked.map, picked.shift_by), (0.5, 40.0));
    }

    #[test]
    fn text_wraps_on_words_within_the_width() {
        let measure = |text: &str| text.len() as f32;
        assert_eq!(
            wrap("one two three four", 9.0, &measure),
            vec!["one two", "three", "four"]
        );
    }
}
