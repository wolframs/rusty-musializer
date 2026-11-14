//! The section card's Generate (VG H4): asking a video model for this
//! section's clip, with the price on the button.
//!
//! Opened by "Generate…" beside "Import clip…" (here and in the Scenes
//! flyout); one small block in the card's vocabulary: the prompt, the five
//! picks each with its price per second and what it takes, the length, the
//! two frames it may start and end on (with their posters), what leaves the
//! machine in words, and the button that carries the price. A press arms it;
//! the second, "Send", is the consent, per job, that the Assist rules ask
//! for (`docs/ASSIST_PROVIDER_CONTRACTS.md` §5, invariant 2). The work is the
//! composition root's (`generation::window`), reported back through
//! [`crate::generation::window::CardView`].
use super::*;
use crate::generation::window::{CardView, GenerateOrder};
use crate::posters::{PosterRequest, PosterView};
use musializer_core::generation::catalog::Offer;
use musializer_core::generation::price::{self, Price, Shape};
use musializer_core::generation::request::{FrameImage, GenerationRequest};
use musializer_core::generation::{MAX_PROMPT_BYTES, PICKS};

/// One row of the generate block, in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeneratePart {
    Label,
    Prompt,
    /// The whole prompt, wrapped, under its one-line field.
    PromptWrap,
    ModelLabel,
    Model(usize),
    LengthLabel,
    Length,
    FramesLabel,
    First,
    Last,
    Leaves,
    Action,
    Status,
    Prices,
}

/// Slots in `LOOK_CARDS`, past the clip rows' (2400-2499).
mod generate_slot {
    pub const PROMPT: u32 = 2500;
    pub const MODEL: u32 = 2510;
    pub const LENGTH: u32 = 2520;
    pub const FIRST: u32 = 2560;
    pub const LAST: u32 = 2561;
    pub const ACTION: u32 = 2562;
    pub const BACK: u32 = 2563;
    pub const CANCEL: u32 = 2564;
    pub const PRICES: u32 = 2565;
    pub const FIRST_POSTER: u32 = 2566;
    pub const LAST_POSTER: u32 = 2567;
    pub const CLOSE: u32 = 2568;
}

/// The block's form: what the person has chosen for one section, kept while
/// the block is open.
#[derive(Clone, Debug)]
pub(crate) struct GenerateForm {
    pub cue_id: u64,
    pub prompt: TextField,
    /// Into [`PICKS`].
    pub model: usize,
    pub seconds: u32,
    pub first: bool,
    pub last: bool,
    /// The first press of the button; the second sends.
    pub armed: bool,
    /// `generate=send` (a probe): press Send once it can be pressed.
    pub probe_send: bool,
}

impl GenerateForm {
    pub(crate) fn new(cue_id: u64) -> Self {
        Self {
            cue_id,
            prompt: TextField::new(TextRules {
                max_bytes: MAX_PROMPT_BYTES,
                ascii_only: false,
            }),
            model: 0,
            seconds: 4,
            first: true,
            last: true,
            armed: false,
            probe_send: false,
        }
    }
}

/// The lengths shown as chips; more than this is a slider.
const MAX_LENGTH_CHIPS: usize = 8;

/// The rows, when the block is open for the selected section.
pub(super) fn plan(view: &CardView, form: &GenerateForm, parts: &mut Vec<(Part, Block)>) {
    let row = |part| (Part::Generate(part), Block::Label);
    let gap = (Part::Space, Block::Gap(look_cards::GROUP_GAP));
    let tight = (Part::Space, Block::Gap(4.0));
    let lengths = offer(view, form.model).map_or(0, |offer| offer.durations.len());
    parts.extend([
        gap,
        (Part::Generate(GeneratePart::Label), Block::Buttons),
        (Part::Generate(GeneratePart::Prompt), Block::Field),
        (Part::Generate(GeneratePart::PromptWrap), Block::Lines(3)),
        gap,
        row(GeneratePart::ModelLabel),
    ]);
    for index in 0..PICKS.len() {
        parts.extend([
            (Part::Generate(GeneratePart::Model(index)), Block::Lines(2)),
            tight,
        ]);
    }
    parts.extend([gap, row(GeneratePart::LengthLabel)]);
    parts.push((
        Part::Generate(GeneratePart::Length),
        if lengths > MAX_LENGTH_CHIPS {
            Block::Slider
        } else {
            Block::Grid(lengths.max(1))
        },
    ));
    parts.extend([
        gap,
        row(GeneratePart::FramesLabel),
        (Part::Generate(GeneratePart::First), Block::Lines(3)),
        tight,
        (Part::Generate(GeneratePart::Last), Block::Lines(3)),
        gap,
        (Part::Generate(GeneratePart::Leaves), Block::Lines(5)),
        (Part::Space, Block::Gap(6.0)),
        (Part::Generate(GeneratePart::Action), Block::Buttons),
        (Part::Generate(GeneratePart::Status), Block::Lines(3)),
        (Part::Generate(GeneratePart::Prices), Block::Buttons),
    ]);
}

fn offer(view: &CardView, model: usize) -> Option<&Offer> {
    let pick = PICKS.get(model)?;
    view.catalog.as_ref()?.offer(pick.id)
}

/// What the block's request would be, without its frames' bytes.
fn draft(
    view: &CardView,
    form: &GenerateForm,
    output: (u32, u32),
    takes_first: bool,
    takes_last: bool,
) -> Option<(GenerationRequest, Shape, Price)> {
    let offer = offer(view, form.model)?;
    let shape = price::shape(offer, output).ok()?;
    let mut request =
        GenerationRequest::new(&offer.id, form.prompt.edit.text(), form.seconds, &shape);
    let image = || FrameImage {
        sha256: String::new(),
        width: shape.size.0,
        height: shape.size.1,
    };
    request.first = (form.first && takes_first).then(image);
    request.last = (form.last && takes_last).then(image);
    let estimate = price::estimate(
        view.catalog.as_deref(),
        crate::generation::now_unix(),
        &offer.id,
        form.seconds,
        &shape,
        request.frame_count(),
    );
    Some((request, shape, estimate))
}

/// The per-second price of a pick at this project's output, in a few words.
fn pick_price(view: &CardView, offer: Option<&Offer>, output: (u32, u32)) -> String {
    let Some(offer) = offer else {
        return "price unknown".into();
    };
    let Ok(shape) = price::shape(offer, output) else {
        return "not this shape".into();
    };
    match price::estimate(
        view.catalog.as_deref(),
        crate::generation::now_unix(),
        &offer.id,
        1,
        &shape,
        0,
    ) {
        Price::Known(estimate) => format!(
            "{}${:.2}/s at {}",
            if estimate.rate.approximate { "~" } else { "" },
            estimate.rate.usd_per_second,
            shape.resolution
        ),
        Price::Unknown(_) => "price unknown".into(),
    }
}

/// The `generate=` fields of the `look cards:` report.
pub(super) fn report(
    view: &CardView,
    form: Option<&GenerateForm>,
    cue_id: u64,
    output: (u32, u32),
) -> String {
    let Some(form) = form.filter(|form| form.cue_id == cue_id) else {
        return "generate=closed".into();
    };
    let price = draft(view, form, output, true, true).map_or_else(
        || "unknown".to_owned(),
        |(_, _, estimate)| estimate.short().replace(' ', "-"),
    );
    format!(
        "generate=open generate-price={price} generate-key={} generate-model={} generate-seconds={} generate-first={} generate-last={} generate-armed={} generate-prompt-bytes={} generate-running={}",
        if view.key.is_ok() { "stored" } else { "none" },
        PICKS[form.model].id,
        form.seconds,
        form.first,
        form.last,
        form.armed,
        form.prompt.edit.text().len(),
        view.running_for(cue_id).map_or("no".to_owned(), |running| running.stage.replace(' ', "-")),
    )
}

/// What a generate row draws with.
pub(super) struct Context<'a, 'b> {
    pub track: &'a Track,
    pub input: &'a ShellInput<'b>,
    pub index: usize,
    pub cue: &'a SceneSwitchCue,
    pub place: &'a Placed,
    pub enabled: bool,
    pub controls: &'a mut Vec<String>,
    pub commands: &'a mut Vec<ShellCommand>,
}

impl Shell {
    /// One row of the generate block.
    #[allow(
        clippy::too_many_lines,
        reason = "the block's rows in drawing order; each arm is short"
    )]
    pub(super) fn generate_part(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        part: GeneratePart,
        cx: &mut Context<'_, '_>,
    ) {
        let Some(mut form) = self
            .look_column
            .cards
            .generate
            .take()
            .filter(|form| form.cue_id == cx.cue.id)
        else {
            return;
        };
        let view = self.generation.clone();
        let rect = cx.place.rect;
        let track = cx.track;
        let output = (track.render_config.width, track.render_config.height);
        let cues = track.scene_switches.cues();
        let next = cues.get(cx.index + 1);
        let measure = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
        let id = |slot: u32| widgets::widget_id(widgets::id::LOOK_CARDS, slot);
        let chosen_offer = offer(&view, form.model).cloned();
        let takes_first = chosen_offer.as_ref().is_none_or(|offer| offer.first_frame);
        let takes_last = chosen_offer.as_ref().is_none_or(|offer| offer.last_frame);
        // This section's own picture: a Video Clip section with no clip has
        // only its placeholder card to start from.
        let own_picture = cx.cue.scene_index != SceneId::Clip.index() as u32
            || track
                .scene_switches
                .clip_section(cx.index, track.base_clip)
                .is_some();
        let running = view.running_for(cx.cue.id).cloned();
        let line = |n: f32| rect.y + n * look_cards::LINE_HEIGHT + 1.0;
        match part {
            GeneratePart::Label => {
                Self::card_label(
                    d,
                    font,
                    UiRect::new(
                        rect.x,
                        rect.y + 4.0,
                        rect.width - 70.0,
                        look_cards::LABEL_HEIGHT,
                    ),
                    "Generate a clip",
                    &format!("section {}", cx.index + 1),
                );
                let close = UiRect::new(rect.x + rect.width - 62.0, rect.y, 62.0, rect.height);
                let state = self.widgets.text_button(
                    d,
                    font,
                    id(generate_slot::CLOSE),
                    close,
                    "Close",
                    false,
                    widgets::ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
                );
                self.widgets.hint(
                    d,
                    state,
                    id(generate_slot::CLOSE),
                    close,
                    "Close the block; what you wrote stays until another section is chosen",
                );
                cx.controls
                    .push(format!("generate-close@{}", rect_text(close)));
                if state.clicked {
                    // Dropped below: the form is not put back.
                    return;
                }
            }
            GeneratePart::Prompt => {
                let probe = self.look_column.cards.probe_text.take();
                if form.prompt.is_focused() {
                    if let Some((Some(value), _)) = &probe {
                        form.prompt.bind(value);
                        form.prompt.set_focused(true);
                    }
                } else if probe.is_some() {
                    self.look_column.cards.probe_text = probe;
                }
                let state = self.widgets.button(d, id(generate_slot::PROMPT), rect);
                if cx.enabled && state.clicked {
                    form.prompt.set_focused(true);
                }
                if !cx.enabled {
                    form.prompt.set_focused(false);
                }
                widgets::fill(d, rect, color::ui_surface());
                form.prompt.draw_with_face(
                    d,
                    AuthoredText::from_ui(font),
                    rect,
                    metric::UI_FONT_VALUE,
                    "What the clip shows: a slow push through fog onto a lit door…",
                    cx.enabled,
                );
                if d.is_key_pressed(KeyboardKey::KEY_ESCAPE)
                    || d.is_key_pressed(KeyboardKey::KEY_ENTER)
                {
                    form.prompt.set_focused(false);
                }
                self.widgets.hint(
                    d,
                    state,
                    id(generate_slot::PROMPT),
                    rect,
                    "Describe the shot: subject, motion, light, camera. It goes to the model as written; no names or handles are needed",
                );
                cx.controls.push(format!(
                    "generate-prompt@{}{}",
                    if cx.enabled { "" } else { "disabled:" },
                    rect_text(rect)
                ));
                if form.armed && state.clicked {
                    form.armed = false;
                }
            }
            GeneratePart::PromptWrap => {
                let text = form.prompt.edit.text();
                // The whole prompt only when the field cannot show it all.
                let fits = measure(text) + 24.0 <= rect.width;
                let (words, ink) = if text.trim().is_empty() || fits {
                    (
                        "Starts on this section's opening picture and ends on the next one's, so the clip bridges two of the video's own frames.".to_owned(),
                        color::ui_muted(),
                    )
                } else {
                    (text.to_owned(), color::ui_ink())
                };
                let lines = wrap(&words, rect.width, &measure);
                for (n, words) in lines.iter().take(3).enumerate() {
                    let last = n == 2 && lines.len() > 3;
                    widgets::draw_text(
                        d,
                        font,
                        &if last {
                            format!("{words}\u{2026}")
                        } else {
                            words.clone()
                        },
                        rect.x,
                        line(n as f32),
                        metric::UI_FONT_CAPTION,
                        ink,
                    );
                }
            }
            GeneratePart::ModelLabel => {
                let label = PICKS[form.model].label;
                Self::card_label(d, font, rect, "Model", label);
            }
            GeneratePart::Model(index) => {
                let pick = &PICKS[index];
                let listed = offer(&view, index);
                let chosen = form.model == index;
                let widget = id(generate_slot::MODEL + index as u32);
                let state = self.widgets.button(d, widget, rect);
                let hovered = cx.enabled && state.hovered;
                let (fill, edge) = if chosen {
                    (color::ui_selected(), color::ui_selected_edge())
                } else if hovered {
                    (color::ui_hover(), color::ui_control_edge())
                } else {
                    (color::ui_control(), color::ui_control_edge())
                };
                widgets::control_box(d, rect, fill, edge, 1.0);
                let ink = if cx.enabled {
                    color::ui_ink()
                } else {
                    color::ui_disabled()
                };
                let price_text = pick_price(&view, listed, output);
                let price_width = measure(&price_text);
                widgets::draw_text_faded(
                    d,
                    font,
                    pick.label,
                    rect.x + 8.0,
                    rect.y + 2.0,
                    (rect.width - price_width - 24.0).max(0.0),
                    12.0,
                    metric::UI_FONT_CAPTION,
                    ink,
                );
                widgets::draw_text(
                    d,
                    font,
                    &price_text,
                    rect.x + rect.width - price_width - 8.0,
                    rect.y + 2.0,
                    metric::UI_FONT_CAPTION,
                    if price_text.starts_with("price unknown") {
                        color::ui_warning()
                    } else {
                        ink
                    },
                );
                let detail = match listed {
                    Some(offer) => format!(
                        "{} \u{00b7} {} \u{00b7} {}",
                        offer.durations_text(),
                        offer.frames_text(),
                        pick.why
                    ),
                    None => format!("not on the price list \u{00b7} {}", pick.why),
                };
                widgets::draw_text_faded(
                    d,
                    font,
                    &ascii_fallback(font.all_loaded(), &detail),
                    rect.x + 8.0,
                    rect.y + 2.0 + look_cards::LINE_HEIGHT,
                    (rect.width - 16.0).max(0.0),
                    16.0,
                    metric::UI_FONT_CAPTION,
                    color::ui_muted(),
                );
                self.card_hint(
                    d,
                    state,
                    widget,
                    rect,
                    &format!("{}: {}", pick.id, pick.why),
                );
                cx.controls.push(format!(
                    "generate-model-{index}@{}{}",
                    if cx.enabled { "" } else { "disabled:" },
                    rect_text(rect)
                ));
                if cx.enabled && state.clicked && !chosen {
                    form.model = index;
                    form.armed = false;
                    // The nearest length the new model makes.
                    if let Some(offer) = listed {
                        if !offer.durations.contains(&form.seconds) {
                            form.seconds = offer
                                .durations
                                .iter()
                                .copied()
                                .min_by_key(|length| length.abs_diff(form.seconds))
                                .unwrap_or(form.seconds);
                        }
                    }
                }
            }
            GeneratePart::LengthLabel => {
                Self::card_label(d, font, rect, "Length", &format!("{} s", form.seconds));
            }
            GeneratePart::Length => {
                let lengths = chosen_offer
                    .as_ref()
                    .map(|offer| offer.durations.clone())
                    .unwrap_or_default();
                if lengths.is_empty() {
                    widgets::draw_text(
                        d,
                        font,
                        "Read today's prices to see this model's lengths",
                        rect.x,
                        rect.y + 4.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                } else if lengths.len() > MAX_LENGTH_CHIPS {
                    let low = lengths[0] as f32;
                    let high = *lengths.last().unwrap_or(&lengths[0]) as f32;
                    let result = self.widgets.slider_row(
                        d,
                        font,
                        widgets::id::LOOK_CARDS,
                        generate_slot::LENGTH,
                        rect,
                        &widgets::SliderRow {
                            label: "Seconds",
                            value_text: &format!("{} s", form.seconds),
                            normalized: (form.seconds as f32 - low) / (high - low).max(1.0),
                            default: None,
                            changed: false,
                            route: None,
                            label_width: 70.0,
                        },
                    );
                    cx.controls
                        .push(format!("generate-length@{}", rect_text(result.layout.bar)));
                    if let (true, Some(unit)) = (cx.enabled, result.drag) {
                        let wanted = (low + unit * (high - low)).round() as u32;
                        form.seconds = lengths
                            .iter()
                            .copied()
                            .min_by_key(|length| length.abs_diff(wanted))
                            .unwrap_or(form.seconds);
                        form.armed = false;
                    }
                } else {
                    for (i, (length, chip)) in lengths.iter().zip(&cx.place.chips).enumerate() {
                        let widget = id(generate_slot::LENGTH + i as u32);
                        let state = self.card_chip(
                            d,
                            font,
                            widget,
                            *chip,
                            &format!("{length} s"),
                            form.seconds == *length,
                            cx.enabled,
                        );
                        self.card_hint(d, state, widget, *chip, &format!("A {length}-second clip"));
                        cx.controls
                            .push(format!("generate-length-{length}@{}", rect_text(*chip)));
                        if cx.enabled && state.clicked {
                            form.seconds = *length;
                            form.armed = false;
                        }
                    }
                }
            }
            GeneratePart::FramesLabel => {
                Self::card_label(d, font, rect, "Frames", "rendered here, look applied");
            }
            GeneratePart::First | GeneratePart::Last => {
                let first = part == GeneratePart::First;
                let (poster_index, offered, reason) = if first {
                    (
                        Some(cx.index),
                        takes_first && own_picture,
                        if !takes_first {
                            "this model takes no first frame"
                        } else if !own_picture {
                            "this section has no picture of its own yet"
                        } else {
                            ""
                        },
                    )
                } else {
                    (
                        next.map(|_| cx.index + 1),
                        takes_last && next.is_some(),
                        if !takes_last {
                            "this model takes no last frame"
                        } else if next.is_none() {
                            "there is no next section"
                        } else {
                            ""
                        },
                    )
                };
                let on = offered && if first { form.first } else { form.last };
                let poster_height = rect.height;
                let poster_width = (poster_height * 16.0 / 9.0).round().min(rect.width * 0.38);
                let poster = UiRect::new(rect.x, rect.y, poster_width, poster_height);
                widgets::fill(d, poster, color::ui_surface());
                let slot = cx.input.workspace.current_index();
                let request = poster_index
                    .zip(slot)
                    .and_then(|(index, slot)| PosterRequest::section(track, slot, index));
                if let Some(request) = request {
                    self.poster_requests.push(request);
                    match cx.input.posters.view_request(track, &request) {
                        PosterView::Ready { texture, aspect } => {
                            clip::draw_poster(
                                d,
                                texture,
                                aspect,
                                poster,
                                if on { 1.0 } else { 0.35 },
                            );
                        }
                        PosterView::Stale { texture, aspect } => {
                            clip::draw_poster(d, texture, aspect, poster, 0.3);
                        }
                        _ => {}
                    }
                }
                d.draw_rectangle_lines_ex(widgets::rectangle(poster), 1.0, color::ui_rule());
                let poster_id = id(if first {
                    generate_slot::FIRST_POSTER
                } else {
                    generate_slot::LAST_POSTER
                });
                let hover = self.widgets.button(d, poster_id, poster);
                self.widgets.hint(
                    d,
                    hover,
                    poster_id,
                    poster,
                    "A poster of the frame; the frame sent is rendered at the clip's size through the export's own path",
                );
                let chip = UiRect::new(
                    poster.x + poster.width + 8.0,
                    rect.y + (rect.height - look_cards::CHIP_HEIGHT) * 0.5,
                    (rect.width - poster.width - 8.0).max(0.0),
                    look_cards::CHIP_HEIGHT,
                );
                let label = if first {
                    "Start on this section's first frame"
                } else {
                    "End on the next section's first frame"
                };
                let widget = id(if first {
                    generate_slot::FIRST
                } else {
                    generate_slot::LAST
                });
                let state = self.card_chip(d, font, widget, chip, label, on, cx.enabled && offered);
                let hint = if offered {
                    if first {
                        "The clip opens on the picture this section starts with, so the cut into it stays where it was"
                    } else {
                        "The clip ends on the next section's opening picture, so it hands over without a jump"
                    }
                } else {
                    reason
                };
                if offered {
                    self.card_hint(d, state, widget, chip, hint);
                } else {
                    self.widgets.hint(d, state, widget, chip, hint);
                }
                cx.controls.push(format!(
                    "generate-{}@{}{};generate-{}-poster@{}",
                    if first { "first" } else { "last" },
                    if cx.enabled && offered {
                        ""
                    } else {
                        "disabled:"
                    },
                    rect_text(chip),
                    if first { "first" } else { "last" },
                    rect_text(poster)
                ));
                if cx.enabled && offered && state.clicked {
                    if first {
                        form.first = !form.first;
                    } else {
                        form.last = !form.last;
                    }
                    form.armed = false;
                }
            }
            GeneratePart::Leaves => {
                let words = match draft(&view, &form, output, takes_first && own_picture, takes_last && next.is_some()) {
                    Some((request, _, _)) => request.leaves(),
                    None => "Read today's prices to see what this model makes and what would leave the machine.".to_owned(),
                };
                for (n, words) in wrap(&words, rect.width, &measure)
                    .iter()
                    .take(5)
                    .enumerate()
                {
                    widgets::draw_text(
                        d,
                        font,
                        words,
                        rect.x,
                        line(n as f32),
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
            }
            GeneratePart::Action => {
                let drafted = draft(
                    &view,
                    &form,
                    output,
                    takes_first && own_picture,
                    takes_last && next.is_some(),
                );
                let blocked: Option<String> = if !cx.enabled {
                    Some(FOLLOWING.to_owned())
                } else if let Err(reason) = &view.key {
                    Some(format!("No key: {reason}"))
                } else if running.is_some() {
                    Some("This section is generating; Cancel below stops waiting".to_owned())
                } else if drafted.is_none() {
                    Some(
                        "Read today's prices first: the price is shown before anything is sent"
                            .to_owned(),
                    )
                } else if let Err(error) =
                    musializer_core::generation::request::check_prompt(form.prompt.edit.text())
                {
                    Some(format!("Write a prompt first ({error})"))
                } else {
                    drafted
                        .as_ref()
                        .and_then(|(request, _, _)| {
                            chosen_offer.as_ref().map(|offer| request.check(offer))
                        })
                        .and_then(Result::err)
                        .map(|error| error.to_string())
                };
                let price_text = drafted.as_ref().map_or_else(
                    || "price unknown".to_owned(),
                    |(_, _, estimate)| estimate.short(),
                );
                let label = match drafted.as_ref().map(|(_, _, estimate)| estimate) {
                    Some(Price::Known(estimate)) => {
                        format!("Generate for about {}", estimate.dollars())
                    }
                    _ => "Generate (price unknown)".to_owned(),
                };
                let widget = id(generate_slot::ACTION);
                if let Some(reason) = blocked {
                    form.armed = false;
                    let state = self.widgets.button(d, widget, rect);
                    self.widgets.disabled_button(
                        d,
                        font,
                        rect,
                        &label,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(d, state, widget, rect, &reason);
                    cx.controls
                        .push(format!("generate-action@disabled:{}", rect_text(rect)));
                } else if form.armed {
                    let back_width = 64.0;
                    let send =
                        UiRect::new(rect.x, rect.y, rect.width - back_width - 6.0, rect.height);
                    let back = UiRect::new(
                        rect.x + rect.width - back_width,
                        rect.y,
                        back_width,
                        rect.height,
                    );
                    let state = self.widgets.text_button(
                        d,
                        font,
                        widget,
                        send,
                        &format!("Send to OpenRouter \u{00b7} {price_text}"),
                        false,
                        widgets::ButtonStyle::Primary,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(
                        d,
                        state,
                        widget,
                        send,
                        "Sends the request now and spends about the price shown. OpenRouter cannot cancel a job once sent",
                    );
                    let back_state = self.widgets.text_button(
                        d,
                        font,
                        id(generate_slot::BACK),
                        back,
                        "Back",
                        false,
                        widgets::ButtonStyle::Ghost,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    cx.controls.push(format!(
                        "generate-send@{};generate-back@{}",
                        rect_text(send),
                        rect_text(back)
                    ));
                    let probe_send = std::mem::take(&mut form.probe_send);
                    if back_state.clicked {
                        form.armed = false;
                    } else if state.clicked || probe_send {
                        if let (Some(slot), Some((_, _, estimate))) =
                            (cx.input.workspace.current_index(), drafted.as_ref())
                        {
                            cx.commands.push(ShellCommand::GenerateClip(GenerateOrder {
                                track_slot: slot,
                                cue_id: cx.cue.id,
                                prompt: form.prompt.edit.text().to_owned(),
                                model: PICKS[form.model].id.to_owned(),
                                seconds: form.seconds,
                                first: form.first && takes_first && own_picture,
                                last: form.last && takes_last && next.is_some(),
                                shown: estimate.usd(),
                            }));
                        }
                        form.armed = false;
                    }
                } else {
                    let state = self.widgets.text_button(
                        d,
                        font,
                        widget,
                        rect,
                        &label,
                        false,
                        widgets::ButtonStyle::Primary,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.card_hint(
                        d,
                        state,
                        widget,
                        rect,
                        "Look once more: the next press asks you to send it",
                    );
                    cx.controls
                        .push(format!("generate-action@{}", rect_text(rect)));
                    if state.clicked {
                        form.armed = true;
                        form.prompt.set_focused(false);
                    }
                }
            }
            GeneratePart::Status => {
                if let Some(running) = &running {
                    let seconds = running.elapsed.as_secs();
                    let words = format!(
                        "{} \u{00b7} {}:{:02} \u{00b7} {} s of {}",
                        if running.cancelling {
                            "stopping"
                        } else {
                            &running.stage
                        },
                        seconds / 60,
                        seconds % 60,
                        running.seconds,
                        running.model
                    );
                    widgets::draw_text_faded(
                        d,
                        font,
                        &ascii_fallback(font.all_loaded(), &words),
                        rect.x,
                        line(0.0),
                        (rect.width - 80.0).max(0.0),
                        16.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_ink(),
                    );
                    let cancel = UiRect::new(
                        rect.x + rect.width - 72.0,
                        rect.y,
                        72.0,
                        look_cards::BUTTON_HEIGHT,
                    );
                    let state = self.widgets.text_button(
                        d,
                        font,
                        id(generate_slot::CANCEL),
                        cancel,
                        "Cancel",
                        false,
                        widgets::ButtonStyle::Ghost,
                        Some(metric::UI_FONT_CAPTION),
                    );
                    self.widgets.hint(
                        d,
                        state,
                        id(generate_slot::CANCEL),
                        cancel,
                        "Stop waiting. A job already sent may still finish and be billed; Generate again with the same settings picks it up",
                    );
                    cx.controls
                        .push(format!("generate-cancel@{}", rect_text(cancel)));
                    if state.clicked && !running.cancelling {
                        cx.commands.push(ShellCommand::CancelGeneration(cx.cue.id));
                    }
                    if let Some(usd) = running.estimate {
                        widgets::draw_text(
                            d,
                            font,
                            &format!(
                                "about ${:.2}; the window stays yours meanwhile",
                                price::ceil_cents(usd)
                            ),
                            rect.x,
                            line(1.0) + 4.0,
                            metric::UI_FONT_CAPTION,
                            color::ui_muted(),
                        );
                    }
                } else if let Some(last) = view.last.get(&cx.cue.id) {
                    let ink = match last.severity {
                        Severity::Error => color::ui_danger(),
                        Severity::Warning => color::ui_warning(),
                        _ => color::ui_success(),
                    };
                    for (n, words) in wrap(&last.words, rect.width, &measure)
                        .iter()
                        .take(3)
                        .enumerate()
                    {
                        widgets::draw_text(
                            d,
                            font,
                            &ascii_fallback(font.all_loaded(), words),
                            rect.x,
                            line(n as f32),
                            metric::UI_FONT_CAPTION,
                            ink,
                        );
                    }
                } else if let Err(reason) = &view.key {
                    for (n, words) in wrap(&format!("No key: {reason}."), rect.width, &measure)
                        .iter()
                        .take(3)
                        .enumerate()
                    {
                        widgets::draw_text(
                            d,
                            font,
                            words,
                            rect.x,
                            line(n as f32),
                            metric::UI_FONT_CAPTION,
                            color::ui_warning(),
                        );
                    }
                }
            }
            GeneratePart::Prices => {
                let button_width = 150.0f32.min(rect.width * 0.5);
                let words = if view.refreshing {
                    "reading today's prices…".to_owned()
                } else {
                    view.prices.clone()
                };
                widgets::draw_text_faded(
                    d,
                    font,
                    &ascii_fallback(font.all_loaded(), &words),
                    rect.x,
                    rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5,
                    (rect.width - button_width - 8.0).max(0.0),
                    16.0,
                    metric::UI_FONT_CAPTION,
                    color::ui_muted(),
                );
                let button = UiRect::new(
                    rect.x + rect.width - button_width,
                    rect.y,
                    button_width,
                    rect.height,
                );
                let widget = id(generate_slot::PRICES);
                let state = self.widgets.text_button(
                    d,
                    font,
                    widget,
                    button,
                    "Read today's prices",
                    false,
                    widgets::ButtonStyle::Ghost,
                    Some(metric::UI_FONT_CAPTION),
                );
                let hint = format!(
                    "Reads OpenRouter's public price list (no key, nothing about you); prices older than a week are unknown. Generated clips are kept in {}",
                    view.kept_in
                        .as_ref()
                        .map_or_else(|| "no folder (set HOME)".to_owned(), |path| path.display().to_string())
                );
                self.widgets.hint(d, state, widget, button, &hint);
                cx.controls
                    .push(format!("generate-prices@{}", rect_text(button)));
                if state.clicked && !view.refreshing {
                    cx.commands.push(ShellCommand::ReadVideoPrices);
                }
            }
        }
        self.look_column.cards.generate = Some(form);
    }
}

impl Shell {
    /// Look, scoped to section `cue_id`, with its generate block open: the
    /// Scenes flyout's Generate… (VG H4).
    pub(crate) fn open_generate_block(&mut self, cue_id: u64) {
        self.set_column_view(crate::ui::shell::ColumnView::Look);
        self.look_column.cards.scope = CardScope::Section;
        self.scene_lane.selected_id = cue_id;
        if self
            .look_column
            .cards
            .generate
            .as_ref()
            .is_none_or(|form| form.cue_id != cue_id)
        {
            self.look_column.cards.generate = Some(GenerateForm::new(cue_id));
        }
    }
}

impl Shell {
    /// `generate=` (a probe, VG H4): the selected section's block open, with
    /// `prompt`, and armed or sent as `mode` says.
    pub(crate) fn probe_generate(&mut self, mode: &str, prompt: Option<&str>) {
        let mut form = GenerateForm::new(self.scene_lane.selected_id);
        if let Some(prompt) = prompt {
            form.prompt.bind(prompt);
        }
        form.armed = matches!(mode, "armed" | "send");
        form.probe_send = mode == "send";
        self.look_column.cards.generate = Some(form);
    }
}
