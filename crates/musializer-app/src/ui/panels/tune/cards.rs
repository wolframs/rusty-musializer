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
    IntroLength,
    OutroLength,
    Bloom,
    BloomThreshold,
    BloomRadius,
    Shoulder,
    Tone,
    Vignette,
    Grain,
    Shadow,
    Highlight,
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
            Self::IntroLength => "intro-length",
            Self::OutroLength => "outro-length",
            Self::Bloom => "bloom",
            Self::BloomThreshold => "bloom-threshold",
            Self::BloomRadius => "bloom-radius",
            Self::Shoulder => "shoulder",
            Self::Tone => "tone",
            Self::Vignette => "vignette",
            Self::Grain => "grain",
            Self::Shadow => "shadow",
            Self::Highlight => "highlight",
        }
    }
}

/// How a knob prints its value, and reads one typed back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    /// `0.45 s`; zero is `none` (a glide).
    Seconds,
    /// A `0..=1` amount as `82%`.
    Percent,
    Degrees,
    SignedDegrees,
    /// Two decimals (bloom, `0..=2`).
    Plain,
    /// A share of the frame's short edge, as `1.6%`.
    ShortEdge,
}

impl Unit {
    fn show(self, value: f32) -> String {
        match self {
            Self::Seconds if value <= 0.0 => "none".to_owned(),
            Self::Seconds => format!("{value:.2} s"),
            Self::Percent => format!("{:.0}%", value * 100.0),
            Self::Degrees => format!("{value:.0}\u{00b0}"),
            Self::SignedDegrees => format!("{value:+.0}\u{00b0}"),
            Self::Plain => format!("{value:.2}"),
            Self::ShortEdge => format!("{:.1}%", value * 100.0),
        }
    }

    /// What the field holds when typing opens: the number, no unit.
    fn edit_text(self, value: f32) -> String {
        match self {
            Self::Seconds => format!("{value:.2}"),
            Self::Percent => format!("{:.0}", value * 100.0),
            Self::Degrees | Self::SignedDegrees => format!("{value:.0}"),
            Self::Plain => format!("{value:.2}"),
            Self::ShortEdge => format!("{:.1}", value * 100.0),
        }
    }

    /// A typed value in this unit, as the model's number; `None` when it is
    /// not a number. Bounds are the knob's.
    fn parse(self, text: &str) -> Option<f32> {
        let text = text.trim();
        if self == Self::Seconds && text.eq_ignore_ascii_case("none") {
            return Some(0.0);
        }
        let text = text.trim_end_matches(['%', '\u{00b0}']).trim();
        let number = if self == Self::Seconds {
            look_cards::parse_seconds(text)?
        } else {
            let number: f32 = text.parse().ok()?;
            number.is_finite().then_some(number)?
        };
        Some(match self {
            Self::Percent | Self::ShortEdge => number / 100.0,
            _ => number,
        })
    }

    /// The model's value at the readout's resolution, so what is stored is
    /// what the row prints.
    fn snap(self, value: f32) -> f32 {
        let step = match self {
            Self::Seconds | Self::Percent | Self::Plain => 0.01,
            Self::Degrees | Self::SignedDegrees => 1.0,
            Self::ShortEdge => 0.001,
        };
        (value / step).round() * step
    }
}

/// How a knob's track maps to its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scale {
    Linear,
    /// [`look_cards::transition_unit`]'s logarithmic track.
    Transition,
    /// [`look_cards::glide_unit`]'s quadratic one.
    Glide,
}

/// One slider row of a card.
#[derive(Clone, Copy, Debug)]
struct Knob {
    label: &'static str,
    field: CardField,
    first: u32,
    value: f32,
    low: f32,
    high: f32,
    default: f32,
    unit: Unit,
    scale: Scale,
}

impl Knob {
    fn unit_of(&self, value: f32) -> f32 {
        match self.scale {
            Scale::Linear => ((value - self.low) / (self.high - self.low)).clamp(0.0, 1.0),
            Scale::Transition => look_cards::transition_unit(value),
            Scale::Glide => look_cards::glide_unit(value),
        }
    }

    fn value_at(&self, unit: f32) -> f32 {
        match self.scale {
            Scale::Linear => self
                .unit
                .snap(self.low + unit.clamp(0.0, 1.0) * (self.high - self.low))
                .clamp(self.low, self.high),
            Scale::Transition => look_cards::transition_seconds(unit),
            Scale::Glide => look_cards::glide_seconds(unit),
        }
    }

    fn typed(&self, text: &str) -> Option<f32> {
        let value = self.unit.parse(text)?;
        Some(
            self.unit
                .snap(value.clamp(self.low, self.high))
                .clamp(self.low, self.high),
        )
    }
}

/// A finish member's knob.
#[allow(
    clippy::too_many_arguments,
    reason = "a knob's whole description, at its one call site"
)]
fn finish_knob(
    label: &'static str,
    field: CardField,
    first: u32,
    value: f32,
    (low, high): (f32, f32),
    default: f32,
    unit: Unit,
) -> Knob {
    Knob {
        label,
        field,
        first,
        value,
        low,
        high,
        default,
        unit,
        scale: Scale::Linear,
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
    pub const PLAY_END: u32 = 14;
    pub const INTRO_KIND: u32 = 160;
    pub const INTRO_LENGTH: u32 = 180;
    pub const OUTRO_KIND: u32 = 190;
    pub const OUTRO_LENGTH: u32 = 210;
    pub const FINISH: u32 = 220;
    pub const BLOOM: u32 = 230;
    pub const BLOOM_THRESHOLD: u32 = 234;
    pub const BLOOM_RADIUS: u32 = 238;
    pub const SHOULDER: u32 = 242;
    pub const TONE: u32 = 246;
    pub const VIGNETTE: u32 = 250;
    pub const GRAIN: u32 = 254;
    /// Six swatches and the hex chip at `+ 8`.
    pub const SHADOW: u32 = 260;
    pub const HIGHLIGHT: u32 = 270;
    /// Hover targets for rows drawn disabled, so their reason can be read:
    /// `+ CardField as u32`.
    pub const DISABLED: u32 = 900;
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
    VideoTitle,
    IntroLabel,
    IntroKinds,
    IntroNone,
    IntroLength,
    OutroLabel,
    OutroKinds,
    OutroLength,
    FinishLabel,
    FinishPresets,
    Bloom,
    BloomThreshold,
    BloomRadius,
    Shoulder,
    Tone,
    Shadow,
    Highlight,
    Vignette,
    Grain,
    Space,
}

/// The finish's two starting points, by name.
const FINISH_PRESETS: [&str; 2] = ["None", "Film"];
/// Shadow and highlight tones a toning can lean toward: the film finish's
/// own first, then a few that hold over most palettes.
const SHADOW_TONES: [u32; 6] = [0x0b2a30, 0x101a3a, 0x2a1030, 0x10261a, 0x2a1a10, 0x000000];
const HIGHLIGHT_TONES: [u32; 6] = [0xffd9a0, 0xfff0d8, 0xffc0a0, 0xd8f0ff, 0xf0d8ff, 0xffffff];

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

/// A disabled row's reason while a treatment is followed.
const FOLLOWING: &str = "Following a treatment: the file decides. Stop following to edit here.";
/// The length row's reason on a cut.
const CUT_HAS_NO_LENGTH: &str = "A cut has no length: pick how the section arrives first.";

/// A transition length's knob.
fn transition_knob(
    label: &'static str,
    field: CardField,
    first: u32,
    transition: Transition,
) -> Knob {
    Knob {
        label,
        field,
        first,
        value: if transition.is_cut() {
            DEFAULT_SECONDS
        } else {
            transition.seconds
        },
        low: musializer_core::project::transition::MIN_SECONDS,
        high: musializer_core::project::transition::MAX_SECONDS,
        default: DEFAULT_SECONDS,
        unit: Unit::Seconds,
        scale: Scale::Transition,
    }
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

    /// Plays `from..until` and stops there when playback was paused: play the
    /// cut, the intro and the end all go through it.
    fn play_span(
        &mut self,
        input: &ShellInput<'_>,
        from: f64,
        until: f64,
        commands: &mut Vec<ShellCommand>,
    ) {
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
        let cues = track.scene_switches.cues();
        if self.look_column.cards.scope == CardScope::Video {
            let gap = (Part::Space, Block::Gap(look_cards::GROUP_GAP));
            let small = (Part::Space, Block::Gap(6.0));
            parts.extend([
                (Part::VideoTitle, Block::Buttons),
                gap,
                (Part::IntroLabel, Block::Label),
            ]);
            if cues.is_empty() {
                parts.push((Part::IntroNone, Block::Lines(2)));
            } else {
                parts.extend([
                    (Part::IntroKinds, Block::Grid(KINDS.len())),
                    small,
                    (Part::IntroLength, Block::Slider),
                ]);
            }
            parts.extend([
                gap,
                (Part::OutroLabel, Block::Label),
                (Part::OutroKinds, Block::Grid(KINDS.len())),
                small,
                (Part::OutroLength, Block::Slider),
                gap,
                (Part::FinishLabel, Block::Label),
                (Part::FinishPresets, Block::Grid(FINISH_PRESETS.len())),
                small,
                (Part::Bloom, Block::Slider),
                (Part::BloomThreshold, Block::Slider),
                (Part::BloomRadius, Block::Slider),
                (Part::Shoulder, Block::Slider),
                (Part::Tone, Block::Slider),
                (Part::Shadow, Block::Slider),
                (Part::Highlight, Block::Slider),
                (Part::Vignette, Block::Slider),
                (Part::Grain, Block::Slider),
                (Part::Space, Block::Gap(8.0)),
                (Part::Status, Block::Line),
            ]);
            return CardsPlan { parts };
        }
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

    /// The typed-entry field over a value chip, once it is open: draws it,
    /// takes a probe's text, and returns the text on Enter. Escape, or a
    /// press elsewhere, closes it.
    fn card_field(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        field: CardField,
        rect: UiRect,
    ) -> Option<String> {
        let (typing, text) = self.look_column.cards.typing.as_mut()?;
        if *typing != field {
            return None;
        }
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
        widgets::fill(d, rect, color::ui_surface());
        text.draw_with_face(
            d,
            AuthoredText::from_ui(font),
            rect,
            metric::UI_FONT_VALUE,
            "",
            true,
        );
        let typed = commit.then(|| text.edit.text().to_owned());
        if commit || cancel || !text.is_focused() {
            self.look_column.cards.typing = None;
        }
        typed
    }

    /// Opens typed entry on `field`, holding `text`.
    fn open_card_field(&mut self, field: CardField, text: &str) {
        let mut entry = TextField::new(TextRules::ascii_query(10));
        entry.bind(text);
        entry.set_focused(true);
        self.look_column.cards.typing = Some((field, entry));
    }

    /// One slider row: a drag moves the value along the knob's scale, a press
    /// on the value types one. Drawn disabled with `disabled` as its reason
    /// when there is one. Returns the new value when it differs from the
    /// knob's, and adds the row's rects to `controls`.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row: its knob, place, state and report"
    )]
    fn knob(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        row: UiRect,
        knob: &Knob,
        disabled: Option<&str>,
        label_width: f32,
        controls: &mut Vec<String>,
    ) -> Option<f32> {
        let shown = ascii_fallback(font.all_loaded(), &knob.unit.show(knob.value));
        if let Some(reason) = disabled {
            let rect = self.disabled_card_row(
                d,
                font,
                knob.field as u32,
                row,
                knob.label,
                &shown,
                reason,
                label_width,
            );
            controls.push(format!(
                "{}@disabled:{}",
                knob.field.name(),
                rect_text(rect)
            ));
            return None;
        }
        let result = self.widgets.slider_row(
            d,
            font,
            widgets::id::LOOK_CARDS,
            knob.first,
            row,
            &widgets::SliderRow {
                label: knob.label,
                value_text: &shown,
                normalized: knob.unit_of(knob.value),
                default: Some(knob.unit_of(knob.default)),
                changed: (knob.value - knob.default).abs() > 0.0005,
                route: None,
                label_width,
            },
        );
        if result.value.clicked {
            self.open_card_field(knob.field, &knob.unit.edit_text(knob.value));
        }
        let typed = self
            .card_field(d, font, knob.field, result.layout.value)
            .and_then(|text| knob.typed(&text));
        controls.push(format!(
            "{}@{};{}-value@{}",
            knob.field.name(),
            rect_text(result.layout.bar),
            knob.field.name(),
            rect_text(result.layout.value)
        ));
        result
            .drag
            .map(|unit| knob.value_at(unit))
            .or(typed)
            .filter(|value| (*value - knob.value).abs() > f32::EPSILON)
    }

    /// The fourteen kinds as a grid of named chips; the press's choice, as a
    /// transition that keeps the length (or takes the default from a cut).
    #[allow(
        clippy::too_many_arguments,
        reason = "a grid: its ids, chips, value and report"
    )]
    fn kind_grid(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        first: u32,
        prefix: &str,
        chips: &[UiRect],
        current: Transition,
        enabled: bool,
        controls: &mut Vec<String>,
    ) -> Option<Transition> {
        let mut picked = None;
        for (i, (kind, chip)) in KINDS.iter().zip(chips).enumerate() {
            let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + i as u32);
            let state = self.card_chip(
                d,
                font,
                id,
                *chip,
                kind_label(*kind),
                current.kind == *kind,
                enabled,
            );
            self.widgets.hint(d, state, id, *chip, kind.look());
            controls.push(format!("{prefix}-{}@{}", kind.name(), rect_text(*chip)));
            if state.clicked && current.kind != *kind {
                let seconds = if current.is_cut() {
                    DEFAULT_SECONDS
                } else {
                    current.seconds
                };
                picked = Some(Transition::new(*kind, seconds));
            }
        }
        picked
    }

    /// A colour row: the label, a few swatches to pick from, and the colour
    /// as `#rrggbb`, which a press types into.
    #[allow(
        clippy::too_many_arguments,
        reason = "one row: its ids, place, value and report"
    )]
    fn colour_row(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        row: UiRect,
        label: &str,
        field: CardField,
        first: u32,
        value: u32,
        presets: &[u32],
        enabled: bool,
        label_width: f32,
        controls: &mut Vec<String>,
    ) -> Option<u32> {
        let layout = widgets::slider_row_layout(row, label_width, false);
        widgets::draw_text(
            d,
            font,
            label,
            layout.label.x + 10.0,
            layout.row.y + (layout.row.height - metric::UI_FONT_CAPTION) * 0.5,
            metric::UI_FONT_CAPTION,
            if enabled {
                color::ui_muted()
            } else {
                color::ui_disabled()
            },
        );
        let size = widgets::SWATCH_SIZE;
        let room = layout.track.width.max(0.0);
        let fit = (((room + 4.0) / (size + 4.0)).floor() as usize).min(presets.len());
        let mut picked = None;
        let top = (layout.row.y + (layout.row.height - size) * 0.5).round();
        let rgba = |rgb: u32| {
            let [r, g, b] = musializer_core::project::look::rgb(rgb);
            let channel = |value: f32| (value * 255.0).round() as u8;
            raylib::prelude::Color::new(channel(r), channel(g), channel(b), 255)
        };
        for (i, preset) in presets.iter().take(fit).enumerate() {
            let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + i as u32);
            let place = UiRect::new(layout.track.x + i as f32 * (size + 4.0), top, size, size);
            let swatch = self
                .widgets
                .swatch(d, id, place, rgba(*preset), *preset == value);
            controls.push(format!("{}-{i}@{}", field.name(), rect_text(swatch.rect)));
            if enabled && swatch.state.clicked && *preset != value {
                picked = Some(*preset);
            }
        }
        // The value: its colour as a small square and its hex, a press types.
        let chip = layout.value;
        let id = widgets::widget_id(widgets::id::LOOK_CARDS, first + 8);
        let state = self.widgets.button(d, id, chip);
        if enabled && state.hovered {
            widgets::control_box(d, chip, color::ui_hover(), color::ui_hover(), 1.0);
        }
        let hex = format!("#{value:06x}");
        let ink = if enabled {
            color::ui_ink()
        } else {
            color::ui_disabled()
        };
        let width = widgets::measure_tabular(font, &hex, metric::UI_FONT_CAPTION);
        widgets::draw_text_tabular(
            d,
            font,
            &hex,
            chip.x + chip.width - width,
            chip.y + (chip.height - metric::UI_FONT_CAPTION) * 0.5,
            metric::UI_FONT_CAPTION,
            ink,
        );
        controls.push(format!("{}-value@{}", field.name(), rect_text(chip)));
        if enabled && state.clicked {
            self.open_card_field(field, &hex);
        }
        if let Some(text) = self.card_field(d, font, field, chip) {
            let digits = text.trim().trim_start_matches('#');
            if digits.len() == 6 {
                if let Ok(typed) = u32::from_str_radix(digits, 16) {
                    picked = Some(typed).filter(|typed| *typed != value);
                }
            }
        }
        if !enabled {
            return None;
        }
        picked
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
        let disabled = (!enabled).then_some(FOLLOWING);
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
                        self.play_span(input, from, until.min(input.duration_seconds), commands);
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
                    if let Some(transition) = self.kind_grid(
                        d,
                        font,
                        slot::KIND,
                        "kind",
                        &place.chips,
                        cue.transition,
                        enabled,
                        &mut controls,
                    ) {
                        edits.push(ScenePlanEdit::SetTransition {
                            cue_id: cue.id,
                            transition,
                        });
                    }
                }
                Part::Length => {
                    let Some((_, cue, _)) = selected else { continue };
                    let knob = transition_knob("Length", CardField::Length, slot::LENGTH, cue.transition);
                    let reason = if cue.transition.is_cut() {
                        Some(CUT_HAS_NO_LENGTH)
                    } else {
                        disabled
                    };
                    if let Some(seconds) =
                        self.knob(d, font, rect, &knob, reason, label_width, &mut controls)
                    {
                        edits.push(ScenePlanEdit::SetTransition {
                            cue_id: cue.id,
                            transition: Transition::new(cue.transition.kind, seconds),
                        });
                    }
                }
                Part::Glide => {
                    let Some((index, cue, previous)) = selected else { continue };
                    let refusal = glide_refusal(index, cue, previous);
                    let knob = Knob {
                        label: "Glide",
                        field: CardField::Glide,
                        first: slot::GLIDE,
                        value: cue.glide,
                        low: 0.0,
                        high: musializer_core::project::scene_switch::MAX_GLIDE_SECONDS,
                        default: 0.0,
                        unit: Unit::Seconds,
                        scale: Scale::Glide,
                    };
                    if let Some(seconds) = self.knob(
                        d,
                        font,
                        rect,
                        &knob,
                        refusal.as_deref().or(disabled),
                        label_width,
                        &mut controls,
                    ) {
                        edits.push(ScenePlanEdit::SetGlide {
                            cue_id: cue.id,
                            seconds,
                        });
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
                    let knob = match part {
                        Part::Map => finish_knob(
                            "Strength",
                            CardField::Map,
                            slot::MAP,
                            grade.map,
                            grade_bounds::MAP,
                            musializer_core::treatment::DEFAULT_GRADE_MAP,
                            Unit::Percent,
                        ),
                        Part::ShiftBy => finish_knob(
                            "Hue turn",
                            CardField::ShiftBy,
                            slot::SHIFT_BY,
                            grade.shift_by,
                            grade_bounds::SHIFT_BY,
                            0.0,
                            Unit::SignedDegrees,
                        ),
                        Part::ShiftFrom => finish_knob(
                            "Turns hues at",
                            CardField::ShiftFrom,
                            slot::SHIFT_FROM,
                            grade.shift_from,
                            grade_bounds::SHIFT_FROM,
                            125.0,
                            Unit::Degrees,
                        ),
                        _ => finish_knob(
                            "Turn width",
                            CardField::ShiftWidth,
                            slot::SHIFT_WIDTH,
                            grade.shift_width,
                            grade_bounds::SHIFT_WIDTH,
                            32.0,
                            Unit::Degrees,
                        ),
                    };
                    if let Some(value) =
                        self.knob(d, font, rect, &knob, disabled, label_width, &mut controls)
                    {
                        let mut next = grade;
                        match part {
                            Part::Map => next.map = value,
                            Part::ShiftBy => next.shift_by = value,
                            Part::ShiftFrom => next.shift_from = value,
                            _ => next.shift_width = value,
                        }
                        edits.push(ScenePlanEdit::SetGrade {
                            cue_id: cue.id,
                            grade: Some(next),
                        });
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
                    let video = self.look_column.cards.scope == CardScope::Video;
                    let (text, tint) = if !track.scene_switches.enabled && video {
                        (
                            "The plan is off: the intro waits for Auto on; the outro and finish play.",
                            color::ui_warning(),
                        )
                    } else if !track.scene_switches.enabled {
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
                Part::VideoTitle => {
                    widgets::draw_text(
                        d,
                        font,
                        "The whole video",
                        rect.x,
                        rect.y + (rect.height - metric::UI_FONT_LABEL) * 0.5,
                        metric::UI_FONT_LABEL,
                        color::ui_ink(),
                    );
                    // Play the end, then Play the intro, from the right.
                    let mut right = rect.x + rect.width;
                    for (label, which) in [("Play the end", slot::PLAY_END), ("Play the intro", slot::PLAY)] {
                        let width = (measure(label) + 20.0).ceil();
                        right -= width;
                        let button = UiRect::new(right, rect.y, width, rect.height);
                        right -= 6.0;
                        let id = widgets::widget_id(widgets::id::LOOK_CARDS, which);
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
                        controls.push(format!(
                            "{}@{}",
                            if which == slot::PLAY { "play-intro" } else { "play-end" },
                            rect_text(button)
                        ));
                        if state.clicked {
                            let (from, until) = if which == slot::PLAY {
                                let seconds = cues.first().map_or(0.0, |cue| {
                                    if cue.transition.is_cut() {
                                        0.0
                                    } else {
                                        musializer_core::project::transition::fitted_seconds(
                                            cue.transition,
                                            cue.end_seconds - cue.start_seconds,
                                        )
                                    }
                                });
                                look_cards::cut_span(0.0, seconds, false, true)
                            } else {
                                let outro = track.scene_switches.outro;
                                let seconds = if outro.is_cut() { 0.0 } else { f64::from(outro.seconds) };
                                (
                                    (input.duration_seconds - seconds - look_cards::CUT_LEAD_SECONDS).max(0.0),
                                    input.duration_seconds,
                                )
                            };
                            self.play_span(input, from, until, commands);
                        }
                    }
                }
                Part::IntroLabel => {
                    let value = cues.first().map_or_else(
                        || "needs a section".to_owned(),
                        |cue| transition_text(cue.transition, "starts on the picture"),
                    );
                    Self::card_label(d, font, rect, "Opens out of black", &ascii_fallback(font.all_loaded(), &value));
                }
                Part::IntroNone => draw_lines(
                    d,
                    rect,
                    "The intro is how the first section arrives: Split here in the plan lane makes one.",
                    color::ui_muted(),
                ),
                Part::IntroKinds => {
                    let Some(first) = cues.first() else { continue };
                    if let Some(transition) = self.kind_grid(
                        d,
                        font,
                        slot::INTRO_KIND,
                        "intro",
                        &place.chips,
                        first.transition,
                        enabled,
                        &mut controls,
                    ) {
                        edits.push(ScenePlanEdit::SetTransition { cue_id: first.id, transition });
                    }
                }
                Part::IntroLength => {
                    let Some(first) = cues.first() else { continue };
                    let knob = transition_knob("Length", CardField::IntroLength, slot::INTRO_LENGTH, first.transition);
                    let reason = if first.transition.is_cut() { Some(CUT_HAS_NO_LENGTH) } else { disabled };
                    if let Some(seconds) = self.knob(d, font, rect, &knob, reason, label_width, &mut controls) {
                        edits.push(ScenePlanEdit::SetTransition {
                            cue_id: first.id,
                            transition: Transition::new(first.transition.kind, seconds),
                        });
                    }
                }
                Part::OutroLabel => {
                    let value = transition_text(track.scene_switches.outro, "stops on the picture");
                    Self::card_label(d, font, rect, "Ends into black", &ascii_fallback(font.all_loaded(), &value));
                }
                Part::OutroKinds => {
                    if let Some(transition) = self.kind_grid(
                        d,
                        font,
                        slot::OUTRO_KIND,
                        "outro",
                        &place.chips,
                        track.scene_switches.outro,
                        enabled,
                        &mut controls,
                    ) {
                        edits.push(ScenePlanEdit::SetOutro(transition));
                    }
                }
                Part::OutroLength => {
                    let outro = track.scene_switches.outro;
                    let knob = transition_knob("Length", CardField::OutroLength, slot::OUTRO_LENGTH, outro);
                    let reason = if outro.is_cut() {
                        Some("A video that stops on the picture has no length: pick how it ends first.")
                    } else {
                        disabled
                    };
                    if let Some(seconds) = self.knob(d, font, rect, &knob, reason, label_width, &mut controls) {
                        edits.push(ScenePlanEdit::SetOutro(Transition::new(outro.kind, seconds)));
                    }
                }
                Part::FinishLabel => {
                    let finish = track.scene_switches.finish;
                    let value = match finish_preset(&finish) {
                        Some(0) => "none",
                        Some(_) => "film",
                        None => "your own",
                    };
                    Self::card_label(d, font, rect, "Finish over every frame", value);
                }
                Part::FinishPresets => {
                    let finish = track.scene_switches.finish;
                    let shown = finish_preset(&finish);
                    for (i, (label, chip)) in FINISH_PRESETS.iter().zip(&place.chips).enumerate() {
                        let id = widgets::widget_id(widgets::id::LOOK_CARDS, slot::FINISH + i as u32);
                        let state = self.card_chip(d, font, id, *chip, label, shown == Some(i), enabled);
                        self.widgets.hint(
                            d,
                            state,
                            id,
                            *chip,
                            if i == 0 {
                                "No finish: the frames as the scenes draw them (the pixel look stays)"
                            } else {
                                "Film: bloom, a highlight shoulder, split toning, a vignette and grain"
                            },
                        );
                        controls.push(format!("finish-{}@{}", label.to_ascii_lowercase(), rect_text(*chip)));
                        if state.clicked && shown != Some(i) {
                            let base = if i == 0 {
                                musializer_core::project::look::Finish::NONE
                            } else {
                                musializer_core::project::look::Finish::FILM
                            };
                            edits.push(ScenePlanEdit::SetFinish(musializer_core::project::look::Finish {
                                pixel: finish.pixel,
                                ..base
                            }));
                        }
                    }
                }
                Part::Bloom
                | Part::BloomThreshold
                | Part::BloomRadius
                | Part::Shoulder
                | Part::Tone
                | Part::Vignette
                | Part::Grain => {
                    use musializer_core::project::look::{finish_bounds, Finish};
                    let finish = track.scene_switches.finish;
                    let none = Finish::NONE;
                    let knob = match part {
                        Part::Bloom => finish_knob("Bloom", CardField::Bloom, slot::BLOOM, finish.bloom, finish_bounds::BLOOM, none.bloom, Unit::Plain),
                        Part::BloomThreshold => finish_knob("Bloom from", CardField::BloomThreshold, slot::BLOOM_THRESHOLD, finish.bloom_threshold, finish_bounds::BLOOM_THRESHOLD, none.bloom_threshold, Unit::Percent),
                        Part::BloomRadius => finish_knob("Bloom spread", CardField::BloomRadius, slot::BLOOM_RADIUS, finish.bloom_radius, finish_bounds::BLOOM_RADIUS, none.bloom_radius, Unit::ShortEdge),
                        Part::Shoulder => finish_knob("Highlights", CardField::Shoulder, slot::SHOULDER, finish.shoulder, finish_bounds::UNIT, none.shoulder, Unit::Percent),
                        Part::Tone => finish_knob("Toning", CardField::Tone, slot::TONE, finish.tone, finish_bounds::UNIT, none.tone, Unit::Percent),
                        Part::Vignette => finish_knob("Vignette", CardField::Vignette, slot::VIGNETTE, finish.vignette, finish_bounds::UNIT, none.vignette, Unit::Percent),
                        _ => finish_knob("Grain", CardField::Grain, slot::GRAIN, finish.grain, finish_bounds::UNIT, none.grain, Unit::Percent),
                    };
                    if let Some(value) = self.knob(d, font, rect, &knob, disabled, label_width, &mut controls) {
                        let mut next = finish;
                        match part {
                            Part::Bloom => next.bloom = value,
                            Part::BloomThreshold => next.bloom_threshold = value,
                            Part::BloomRadius => next.bloom_radius = value,
                            Part::Shoulder => next.shoulder = value,
                            Part::Tone => next.tone = value,
                            Part::Vignette => next.vignette = value,
                            _ => next.grain = value,
                        }
                        edits.push(ScenePlanEdit::SetFinish(next));
                    }
                }
                Part::Shadow | Part::Highlight => {
                    let finish = track.scene_switches.finish;
                    let shadow = *part == Part::Shadow;
                    let picked = self.colour_row(
                        d,
                        font,
                        rect,
                        if shadow { "Shadow tone" } else { "Highlight tone" },
                        if shadow { CardField::Shadow } else { CardField::Highlight },
                        if shadow { slot::SHADOW } else { slot::HIGHLIGHT },
                        if shadow { finish.shadow_rgb } else { finish.highlight_rgb },
                        if shadow { &SHADOW_TONES } else { &HIGHLIGHT_TONES },
                        enabled,
                        label_width,
                        &mut controls,
                    );
                    if let Some(rgb) = picked {
                        let mut next = finish;
                        if shadow {
                            next.shadow_rgb = rgb;
                        } else {
                            next.highlight_rgb = rgb;
                        }
                        edits.push(ScenePlanEdit::SetFinish(next));
                    }
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
        let plan = &track.scene_switches;
        let finish = plan.finish;
        let video_text =
            format!(
            "intro={} outro={} finish={:.2},{:.2},{:.3},{:.2},{:.2},{:.2},{:.2},#{:06x},#{:06x}",
            cues.first().map_or_else(
                || "none".to_owned(),
                |cue| format!("{}:{:.2}", cue.transition.kind.name(), cue.transition.seconds)
            ),
            format_args!("{}:{:.2}", plan.outro.kind.name(), plan.outro.seconds),
            finish.bloom,
            finish.bloom_threshold,
            finish.bloom_radius,
            finish.shoulder,
            finish.tone,
            finish.vignette,
            finish.grain,
            finish.shadow_rgb,
            finish.highlight_rgb,
        );
        self.look_column.cards.report = format!(
            "scope={} target={target_text} {video_text} follow={} editable={} cut={cut} typing={typing} area={} frame={} controls={}{probe}",
            self.look_column.cards.scope.name(),
            followed.as_deref().unwrap_or("none"),
            u8::from(enabled),
            rect_text(area),
            rect_text(frame),
            controls.join(";"),
        );
    }
}

/// Which of [`FINISH_PRESETS`] a finish is, ignoring its pixel look.
fn finish_preset(finish: &musializer_core::project::look::Finish) -> Option<usize> {
    use musializer_core::project::look::Finish;
    let bare = Finish {
        pixel: Finish::NONE.pixel,
        ..*finish
    };
    if !bare.shades() {
        Some(0)
    } else if bare == Finish::FILM {
        Some(1)
    } else {
        None
    }
}

/// How a transition reads in a label: `Fade · 0.60 s`, or `cut` in words.
fn transition_text(transition: Transition, cut: &str) -> String {
    if transition.is_cut() {
        cut.to_owned()
    } else {
        format!(
            "{} \u{00b7} {:.2} s",
            kind_label(transition.kind),
            transition.seconds
        )
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
