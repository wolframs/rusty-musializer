//! The section card's clip rows (F1c): what a treatment says about a Video
//! Clip section's clip — which clip, where it starts, what happens after its
//! end, how it fits — offered in the cards' own vocabulary, with a poster of
//! the frame the section starts on.
//!
//! Every edit is one [`ScenePlanEdit::SetClip`] carrying the section's whole
//! placement, applied by [`apply_clip_edit`] the way a loader would hold it:
//! the section plays Video Clip, its clip is in the project, `from` lies
//! inside it. "Import clip…" is the Scenes flyout's own import
//! (`ShellCommand::ImportClipDialog`), which lands in the section selected in
//! the lane, the one this card edits.
use super::*;
use crate::clips::TrackClip;
use crate::posters::{PosterRequest, PosterSeat, PosterView, ThumbView};
use musializer_core::project::clip::{
    self as clip_model, ClipFit, ClipOverrun, ClipPlacement, ClipRef,
};
use musializer_core::project::scene_switch::ClipSection;

/// One row of the clip block, in drawing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipPart {
    /// "Plays": the clip's name and length.
    Label,
    /// The poster of the section's start beside the clip's name, size and
    /// whose it is, or why there is no picture.
    Head,
    /// One library clip, by index: its middle frame, name and length.
    Choice(usize),
    /// An empty library, in words.
    NoClips,
    /// "Import clip…" and the disabled "Generate…".
    Import,
    StartLabel,
    /// Continue the section before, or an own start.
    StartChoice,
    /// The own start's time in the clip.
    From,
    AfterLabel,
    After,
    FitLabel,
    Fit,
}

/// Slots in `LOOK_CARDS`, past the disabled rows' hover targets (2000 +
/// at most 339).
mod clip_slot {
    pub const CHOICE: u32 = 2400;
    pub const IMPORT: u32 = 2470;
    pub const GENERATE: u32 = 2471;
    pub const CONTINUE: u32 = 2472;
    pub const OWN: u32 = 2473;
    pub const POSTER: u32 = 2474;
    /// Four ids, a slider row.
    pub const FROM: u32 = 2480;
    pub const AFTER: u32 = 2490;
    pub const FIT: u32 = 2495;
}

/// What a clip row draws with.
pub(super) struct Context<'a, 'b> {
    pub track: &'a Track,
    pub input: &'a ShellInput<'b>,
    /// The selected section, by index, and its cue.
    pub index: usize,
    pub cue: &'a SceneSwitchCue,
    pub place: &'a Placed,
    pub enabled: bool,
    pub label_width: f32,
    pub controls: &'a mut Vec<String>,
    pub edits: &'a mut Vec<ScenePlanEdit>,
    pub commands: &'a mut Vec<ShellCommand>,
}

/// The clip rows for the selected section, which plays Video Clip.
pub(super) fn plan(track: &Track, selected: u64, parts: &mut Vec<(Part, Block)>) {
    let row = |part| (Part::Clip(part), Block::Label);
    let gap = (Part::Space, Block::Gap(look_cards::GROUP_GAP));
    let tight = (Part::Space, Block::Gap(4.0));
    let index = track
        .scene_switches
        .cues()
        .iter()
        .position(|cue| cue.id == selected);
    let section = index.and_then(|index| track.scene_switches.clip_section(index, track.base_clip));
    parts.extend([
        row(ClipPart::Label),
        (Part::Clip(ClipPart::Head), Block::Lines(4)),
        tight,
    ]);
    if track.clips.is_empty() {
        parts.push((Part::Clip(ClipPart::NoClips), Block::Lines(2)));
    }
    for choice in 0..track.clips.len() {
        parts.extend([
            (Part::Clip(ClipPart::Choice(choice)), Block::Lines(2)),
            tight,
        ]);
    }
    parts.extend([(Part::Clip(ClipPart::Import), Block::Buttons), gap]);
    // Where it starts, its end and its fit only mean something for a clip
    // the project holds.
    if section.is_some_and(|section| {
        clip_model::find(&asset_list(track), section.placement.clip).is_some()
    }) {
        parts.extend([
            row(ClipPart::StartLabel),
            (Part::Clip(ClipPart::StartChoice), Block::Grid(2)),
            (Part::Space, Block::Gap(6.0)),
            (Part::Clip(ClipPart::From), Block::Slider),
            gap,
            row(ClipPart::AfterLabel),
            (
                Part::Clip(ClipPart::After),
                Block::Grid(ClipOverrun::ALL.len()),
            ),
            gap,
            row(ClipPart::FitLabel),
            (Part::Clip(ClipPart::Fit), Block::Grid(ClipFit::ALL.len())),
            gap,
        ]);
    }
}

fn asset_list(track: &Track) -> Vec<clip_model::ClipAsset> {
    track.clips.iter().map(|clip| clip.asset.clone()).collect()
}

/// Applies a card's clip edit to `track`, or changes nothing: the section
/// plays Video Clip, the clip is one the project holds, and `from` lies
/// inside it (`placement_fits`, the loader's own rule).
pub(crate) fn apply_clip_edit(track: &mut Track, cue_id: u64, clip: ClipPlacement) -> bool {
    let plays_clip = track
        .scene_switches
        .cues()
        .iter()
        .any(|cue| cue.id == cue_id && cue.scene_index == SceneId::Clip.index() as u32);
    plays_clip
        && clip_model::placement_fits(&asset_list(track), &clip)
        && track.scene_switches.set_clip(cue_id, Some(clip))
}

/// The clip a library entry is, for a report: its first eight hex digits.
fn short(clip: ClipRef) -> String {
    clip.short()
}

/// What a clip choice's second line says: its size, or why it cannot draw.
fn choice_words(clip: &TrackClip, thumb: &ThumbView<'_>) -> (String, bool) {
    match thumb {
        ThumbView::Refused(words) => (words.clone(), true),
        _ => (format!("{}x{}", clip.asset.width, clip.asset.height), false),
    }
}

/// The section's start, in words: continuing, its own, or the clip's start.
fn start_words(section: &ClipSection, index: usize, length: f64) -> String {
    let at = section.start_on_screen(length);
    if section.continues {
        format!("goes on from section {index} \u{00b7} {at:.2} s")
    } else if section.placement.from_seconds.is_some() {
        format!("its own \u{00b7} {at:.2} s")
    } else {
        format!("the clip's start \u{00b7} {at:.2} s")
    }
}

fn after_words(after: ClipOverrun) -> &'static str {
    match after {
        ClipOverrun::Hold => "holds its last frame",
        ClipOverrun::Loop => "starts again",
        ClipOverrun::Bounce => "plays back, then on",
    }
}

fn after_label(after: ClipOverrun) -> &'static str {
    match after {
        ClipOverrun::Hold => "Hold",
        ClipOverrun::Loop => "Loop",
        ClipOverrun::Bounce => "Bounce",
    }
}

fn fit_label(fit: ClipFit) -> &'static str {
    match fit {
        ClipFit::Cover => "Cover",
        ClipFit::Contain => "Contain",
    }
}

/// The `clip=` fields of the `look cards:` report for section `index`.
pub(super) fn report(track: &Track, index: usize, cards: &CardState) -> String {
    let Some(section) = track.scene_switches.clip_section(index, track.base_clip) else {
        return "clip=-".to_owned();
    };
    let length = crate::clips::find(&track.clips, section.placement.clip)
        .map_or(0.0, |clip| clip.asset.duration_seconds);
    let from = if section.continues {
        format!("continues-{index}")
    } else if section.placement.from_seconds.is_some() {
        "own".to_owned()
    } else {
        "start".to_owned()
    };
    format!(
        "clip={} clip-whose={} clip-from={from} clip-at={:.4} clip-after={} clip-fit={} clip-library={}{}",
        short(section.placement.clip),
        if section.inherited { "track" } else { "own" },
        section.start_on_screen(length),
        section.placement.after.name(),
        section.placement.fit.name(),
        track.clips.len(),
        cards.clip_drawn,
    )
}

impl Shell {
    /// One clip row of the section card.
    #[allow(
        clippy::too_many_lines,
        reason = "the clip rows in drawing order; each arm is short"
    )]
    pub(super) fn clip_part(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        part: ClipPart,
        cx: &mut Context<'_, '_>,
    ) {
        let rect = cx.place.rect;
        let track = cx.track;
        let section = track.scene_switches.clip_section(cx.index, track.base_clip);
        let clip =
            section.and_then(|section| crate::clips::find(&track.clips, section.placement.clip));
        let slot = cx.input.workspace.current_index();
        let measure = |text: &str| widgets::measure(font, text, metric::UI_FONT_CAPTION);
        let id = |slot: u32| widgets::widget_id(widgets::id::LOOK_CARDS, slot);
        let edit = |cx: &mut Context<'_, '_>, placement: ClipPlacement| {
            if placement != section.map_or(placement, |section| section.placement)
                || section.is_some_and(|section| section.inherited)
            {
                cx.edits.push(ScenePlanEdit::SetClip {
                    cue_id: cx.cue.id,
                    clip: placement,
                });
            }
        };
        match part {
            ClipPart::Label => {
                let value = match track.clips.len() {
                    0 => "none in this project".to_owned(),
                    1 => "1 in this project".to_owned(),
                    count => format!("{count} in this project"),
                };
                Self::card_label(d, font, rect, "Clip", &value);
            }
            ClipPart::Head => {
                // The poster: the section's own seat in the plan lane's service,
                // where the section has arrived.
                let poster_height = rect.height;
                let poster_width = (poster_height * 16.0 / 9.0).round().min(rect.width * 0.45);
                let poster = UiRect::new(rect.x, rect.y, poster_width, poster_height);
                let request = slot.and_then(|slot| PosterRequest::section(track, slot, cx.index));
                let view = request.map(|request| {
                    self.poster_requests.push(request);
                    cx.input.posters.view_request(track, &request)
                });
                widgets::fill(d, poster, color::ui_surface());
                let state = match view {
                    Some(PosterView::Ready { texture, aspect }) => {
                        draw_poster(d, texture, aspect, poster, 1.0);
                        "ready"
                    }
                    Some(PosterView::Stale { texture, aspect }) => {
                        draw_poster(d, texture, aspect, poster, 0.45);
                        "stale"
                    }
                    Some(PosterView::Pending) | None => "pending",
                    Some(PosterView::Unavailable(_)) => "unavailable",
                };
                d.draw_rectangle_lines_ex(widgets::rectangle(poster), 1.0, color::ui_rule());
                let hover = self.widgets.button(d, id(clip_slot::POSTER), poster);
                let poster_hint = match view {
                    Some(PosterView::Unavailable(reason)) => format!("No poster: {reason}"),
                    _ => "The frame this section starts on, once it has arrived".to_owned(),
                };
                self.widgets
                    .hint(d, hover, id(clip_slot::POSTER), poster, &poster_hint);
                cx.controls
                    .push(format!("clip-poster@{}", rect_text(poster)));
                // Beside it: the clip's name, its size and whose it is, or the
                // placeholder's words for why it cannot draw.
                let left = poster.x + poster.width + 8.0;
                let width = (rect.x + rect.width - left).max(0.0);
                let line = |n: f32| rect.y + n * look_cards::LINE_HEIGHT + 1.0;
                let (name, detail, words) = match (section, clip) {
                    (None, _) => (
                        "No clip chosen".to_owned(),
                        String::new(),
                        Some("Pick one below, or Import clip".to_owned()),
                    ),
                    (Some(section), None) => (
                        format!("Clip {}", short(section.placement.clip)),
                        String::new(),
                        Some(format!(
                            "Clip {} is not in this project",
                            short(section.placement.clip)
                        )),
                    ),
                    (Some(section), Some(clip)) => {
                        let refused = match cx.input.posters.view_clip(clip) {
                            ThumbView::Refused(words)
                                if words != "posters are switched off for this run" =>
                            {
                                Some(words)
                            }
                            _ => None,
                        };
                        (
                            clip.asset.name.clone(),
                            format!(
                                "{}x{} \u{00b7} {:.1} s \u{00b7} {}",
                                clip.asset.width,
                                clip.asset.height,
                                clip.asset.duration_seconds,
                                if section.inherited {
                                    "the track's"
                                } else {
                                    "this section's"
                                }
                            ),
                            refused,
                        )
                    }
                };
                widgets::draw_text_faded(
                    d,
                    font,
                    &ascii_fallback(font.all_loaded(), &name),
                    left,
                    line(0.0),
                    width,
                    16.0,
                    metric::UI_FONT_CAPTION,
                    color::ui_ink(),
                );
                if !detail.is_empty() {
                    widgets::draw_text_faded(
                        d,
                        font,
                        &ascii_fallback(font.all_loaded(), &detail),
                        left,
                        line(1.0),
                        width,
                        16.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
                let mut said = String::new();
                if let Some(words) = &words {
                    let first = if detail.is_empty() { 1.0 } else { 2.0 };
                    for (n, text) in wrap(words, width, &measure)
                        .iter()
                        .take(4 - first as usize)
                        .enumerate()
                    {
                        widgets::draw_text(
                            d,
                            font,
                            text,
                            left,
                            line(first + n as f32),
                            metric::UI_FONT_CAPTION,
                            color::ui_warning(),
                        );
                    }
                    said.clone_from(words);
                }
                self.look_column.cards.clip_drawn = format!(
                    " clip-poster={state} clip-words=\"{}\"",
                    said.replace('"', "'")
                );
            }
            ClipPart::NoClips => {
                let text =
                    "This project holds no clips yet: Import clip adds one and plays it here.";
                for (n, words) in wrap(text, rect.width, &measure).iter().take(2).enumerate() {
                    widgets::draw_text(
                        d,
                        font,
                        words,
                        rect.x,
                        rect.y + n as f32 * look_cards::LINE_HEIGHT + 1.0,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
            }
            ClipPart::Choice(choice) => {
                let Some(entry) = track.clips.get(choice) else {
                    return;
                };
                let Some(key) = entry.clip_ref() else { return };
                if let Some(slot) = slot {
                    self.poster_requests.push(PosterRequest {
                        track_slot: slot,
                        seat: PosterSeat::ClipThumb(key),
                        visible: true,
                        run: musializer_core::ui::plan_runs::RunSpan {
                            scene: SceneId::Clip,
                            start_seconds: 0.0,
                            end_seconds: 0.0,
                            first_section: 0,
                            sections: 0,
                            first_cue_id: 0,
                        },
                    });
                }
                let chosen = section.is_some_and(|section| section.placement.clip == key);
                let row = UiRect::new(rect.x, rect.y, rect.width, rect.height);
                let widget = id(clip_slot::CHOICE + choice as u32);
                let state = self.widgets.button(d, widget, row);
                let hovered = cx.enabled && state.hovered;
                let (fill, edge) = if chosen {
                    (color::ui_selected(), color::ui_selected_edge())
                } else if hovered {
                    (color::ui_hover(), color::ui_control_edge())
                } else {
                    (color::ui_control(), color::ui_control_edge())
                };
                widgets::control_box(d, row, fill, edge, 1.0);
                // The middle frame, fitted into a 16:9 well at the left.
                let well = UiRect::new(
                    row.x + 2.0,
                    row.y + 2.0,
                    ((row.height - 4.0) * 16.0 / 9.0).round(),
                    row.height - 4.0,
                );
                widgets::fill(d, well, color::ui_surface());
                let thumb = cx.input.posters.view_clip(entry);
                let thumb_state = match &thumb {
                    ThumbView::Ready { texture, size } => {
                        let scale = (well.width / size.0 as f32).min(well.height / size.1 as f32);
                        let (w, h) = (size.0 as f32 * scale, size.1 as f32 * scale);
                        d.draw_texture_pro(
                            *texture,
                            raylib::prelude::Rectangle::new(0.0, 0.0, size.0 as f32, size.1 as f32),
                            raylib::prelude::Rectangle::new(
                                (well.x + (well.width - w) * 0.5).round(),
                                (well.y + (well.height - h) * 0.5).round(),
                                w.round(),
                                h.round(),
                            ),
                            raylib::prelude::Vector2::zero(),
                            0.0,
                            if cx.enabled {
                                raylib::prelude::Color::WHITE
                            } else {
                                widgets::alpha(raylib::prelude::Color::WHITE, 0.5)
                            },
                        );
                        "ready"
                    }
                    ThumbView::Pending => "pending",
                    ThumbView::Refused(_) => "refused",
                };
                let ink = if !cx.enabled {
                    color::ui_disabled()
                } else if chosen || hovered {
                    color::ui_ink()
                } else {
                    color::ui_muted()
                };
                let length = format!("{:.1} s", entry.asset.duration_seconds);
                let length_width = widgets::measure_tabular(font, &length, metric::UI_FONT_CAPTION);
                let text_left = well.x + well.width + 6.0;
                let right = row.x + row.width - 6.0;
                widgets::draw_text_tabular(
                    d,
                    font,
                    &length,
                    right - length_width,
                    row.y + 2.0,
                    metric::UI_FONT_CAPTION,
                    ink,
                );
                widgets::draw_text_faded(
                    d,
                    font,
                    &ascii_fallback(font.all_loaded(), &entry.asset.name),
                    text_left,
                    row.y + 2.0,
                    (right - length_width - 6.0 - text_left).max(0.0),
                    16.0,
                    metric::UI_FONT_CAPTION,
                    ink,
                );
                let (second, refused) = choice_words(entry, &thumb);
                let second = if chosen && !refused {
                    format!("{second} \u{00b7} plays here")
                } else {
                    second
                };
                widgets::draw_text_faded(
                    d,
                    font,
                    &ascii_fallback(font.all_loaded(), &second),
                    text_left,
                    row.y + 2.0 + look_cards::LINE_HEIGHT - 2.0,
                    (right - text_left).max(0.0),
                    16.0,
                    metric::UI_FONT_MICRO + 1.0,
                    if refused && cx.enabled {
                        color::ui_warning()
                    } else {
                        color::ui_disabled()
                    },
                );
                let hint = if chosen {
                    format!("{}: this section plays it", entry.asset.name)
                } else {
                    format!("Play {} in this section", entry.asset.name)
                };
                self.card_hint(d, state, widget, row, &hint);
                cx.controls.push(format!(
                    "clip-choice-{choice}@{}{}",
                    rect_text(row),
                    if chosen { "(chosen)" } else { "" }
                ));
                cx.controls
                    .push(format!("clip-thumb-{choice}={thumb_state}"));
                if cx.enabled && state.clicked && !chosen {
                    // Another clip: its own start, or the section before's
                    // clock when that plays it; the end and fit stay.
                    let placement = ClipPlacement {
                        clip: key,
                        from_seconds: None,
                        ..section
                            .map_or_else(|| ClipPlacement::new(key), |section| section.placement)
                    };
                    cx.edits.push(ScenePlanEdit::SetClip {
                        cue_id: cx.cue.id,
                        clip: placement,
                    });
                }
            }
            ClipPart::Import => {
                let gap = 4.0;
                let generate_width = (rect.width * 0.42).clamp(0.0, 112.0);
                let import = UiRect::new(
                    rect.x,
                    rect.y,
                    (rect.width - generate_width - gap).max(0.0),
                    rect.height,
                );
                let generate = UiRect::new(
                    rect.x + rect.width - generate_width,
                    rect.y,
                    generate_width,
                    rect.height,
                );
                let import_id = id(clip_slot::IMPORT);
                let state = if cx.enabled {
                    self.widgets.text_button(
                        d,
                        font,
                        import_id,
                        import,
                        "Import clip\u{2026}",
                        false,
                        widgets::ButtonStyle::Neutral,
                        Some(metric::UI_FONT_CAPTION),
                    )
                } else {
                    let state = self.widgets.button(d, import_id, import);
                    self.widgets.disabled_button(
                        d,
                        font,
                        import,
                        "Import clip\u{2026}",
                        Some(metric::UI_FONT_CAPTION),
                    );
                    widgets::ButtonState {
                        clicked: false,
                        ..state
                    }
                };
                self.card_hint(d, state, import_id, import, crate::clips::IMPORT_HINT);
                if state.clicked {
                    cx.commands.push(ShellCommand::ImportClipDialog);
                }
                // A real registration for the hover and to take the press, so
                // a click on the disabled control lands nowhere, as in the
                // flyout's footer.
                let generate_id = id(clip_slot::GENERATE);
                let hover = self.widgets.button(d, generate_id, generate);
                self.widgets.disabled_button(
                    d,
                    font,
                    generate,
                    "Generate\u{2026}",
                    Some(metric::UI_FONT_CAPTION),
                );
                self.widgets.hint(
                    d,
                    hover,
                    generate_id,
                    generate,
                    crate::clips::GENERATE_NOT_BUILT,
                );
                cx.controls.push(format!(
                    "clip-import@{}{};clip-generate@disabled:{}",
                    if cx.enabled { "" } else { "disabled:" },
                    rect_text(import),
                    rect_text(generate)
                ));
            }
            ClipPart::StartLabel => {
                let (Some(section), Some(clip)) = (section, clip) else {
                    return;
                };
                let value = start_words(&section, cx.index, clip.asset.duration_seconds);
                Self::card_label(
                    d,
                    font,
                    rect,
                    "Starts at",
                    &ascii_fallback(font.all_loaded(), &value),
                );
            }
            ClipPart::StartChoice => {
                let (Some(section), Some(clip)) = (section, clip) else {
                    return;
                };
                let length = clip.asset.duration_seconds;
                let previous = cx.index;
                let choices = [
                    (
                        "Continue",
                        clip_slot::CONTINUE,
                        "start-continue",
                        section.continues,
                        section.continuable,
                    ),
                    (
                        "Own start",
                        clip_slot::OWN,
                        "start-own",
                        !section.continues,
                        true,
                    ),
                ];
                for ((label, which, name, chosen, offered), chip) in
                    choices.into_iter().zip(&cx.place.chips)
                {
                    let widget = id(which);
                    let state = self.card_chip(
                        d,
                        font,
                        widget,
                        *chip,
                        label,
                        chosen,
                        cx.enabled && offered,
                    );
                    let hint = if which == clip_slot::CONTINUE {
                        if section.continuable {
                            format!(
                                "Go on with section {previous}'s clock: the clip carries on where it leaves off ({:.2} s here)",
                                section.start_on_screen(length)
                            )
                        } else {
                            format!(
                                "Section {previous} does not play this clip, so there is no clock to go on with"
                            )
                        }
                    } else {
                        "Start at a time of your own: from the frame on screen now, until you change the number"
                            .to_owned()
                    };
                    if offered {
                        self.card_hint(d, state, widget, *chip, &hint);
                    } else {
                        self.widgets.hint(d, state, widget, *chip, &hint);
                    }
                    cx.controls.push(format!(
                        "{name}@{}{}",
                        if cx.enabled && offered {
                            ""
                        } else {
                            "disabled:"
                        },
                        rect_text(*chip)
                    ));
                    if state.clicked && !chosen {
                        let placement = if which == clip_slot::CONTINUE {
                            section.continued()
                        } else {
                            // Pinned where the clock is: nothing moves until
                            // the number does.
                            section.starting_at(section.pinned_start(length))
                        };
                        edit(cx, placement);
                    }
                }
            }
            ClipPart::From => {
                let (Some(section), Some(clip)) = (section, clip) else {
                    return;
                };
                let length = clip.asset.duration_seconds;
                // A `from` lies before the clip's end; at the readout's
                // hundredth of a second.
                let high = (((length - 0.001).max(0.0) * 100.0).floor() / 100.0) as f32;
                let knob = Knob {
                    label: "From",
                    field: CardField::ClipFrom,
                    first: clip_slot::FROM,
                    value: section.start_on_screen(length) as f32,
                    low: 0.0,
                    high,
                    default: 0.0,
                    unit: Unit::ClipSeconds,
                    scale: Scale::Linear,
                };
                let disabled = (!cx.enabled).then_some(FOLLOWING);
                if let Some(seconds) =
                    self.knob(d, font, rect, &knob, disabled, cx.label_width, cx.controls)
                {
                    edit(cx, section.starting_at(f64::from(seconds)));
                }
                // The knob reports itself as `clip-from`; give the bar its
                // clip-specific name too, for a reader of the report.
            }
            ClipPart::AfterLabel => {
                let Some(section) = section else { return };
                Self::card_label(
                    d,
                    font,
                    rect,
                    "After its end",
                    after_words(section.placement.after),
                );
            }
            ClipPart::After => {
                let Some(section) = section else { return };
                for (i, (after, chip)) in ClipOverrun::ALL.iter().zip(&cx.place.chips).enumerate() {
                    let widget = id(clip_slot::AFTER + i as u32);
                    let (state, glyph) = self.glyph_chip(
                        d,
                        font,
                        widget,
                        *chip,
                        after_label(*after),
                        section.placement.after == *after,
                        cx.enabled,
                        |d, place, ink, dim| draw_after_glyph(d, *after, place, ink, dim),
                    );
                    let hint = match after {
                        ClipOverrun::Hold => "When the section outlasts the clip, its last frame stays",
                        ClipOverrun::Loop => "When the section outlasts the clip, it starts again from the top",
                        ClipOverrun::Bounce => "When the section outlasts the clip, it plays backwards to the start, then on again",
                    };
                    self.card_hint(d, state, widget, *chip, hint);
                    cx.controls
                        .push(format!("after-{}@{}", after.name(), rect_text(*chip)));
                    if let Some(glyph) = glyph {
                        cx.controls.push(format!(
                            "after-{}-glyph@{}",
                            after.name(),
                            rect_text(glyph)
                        ));
                    }
                    if state.clicked && section.placement.after != *after {
                        edit(
                            cx,
                            ClipPlacement {
                                after: *after,
                                ..section.placement
                            },
                        );
                    }
                }
            }
            ClipPart::FitLabel => {
                let Some(section) = section else { return };
                let words = match section.placement.fit {
                    ClipFit::Cover => "fills the frame, cropped",
                    ClipFit::Contain => "the whole clip, black around it",
                };
                Self::card_label(d, font, rect, "Fit", words);
            }
            ClipPart::Fit => {
                let Some(section) = section else { return };
                for (i, (fit, chip)) in ClipFit::ALL.iter().zip(&cx.place.chips).enumerate() {
                    let widget = id(clip_slot::FIT + i as u32);
                    let (state, glyph) = self.glyph_chip(
                        d,
                        font,
                        widget,
                        *chip,
                        fit_label(*fit),
                        section.placement.fit == *fit,
                        cx.enabled,
                        |d, place, ink, dim| draw_fit_glyph(d, *fit, place, ink, dim),
                    );
                    let hint = match fit {
                        ClipFit::Cover => "Fill the frame, cropping the clip's long side",
                        ClipFit::Contain => {
                            "Show the whole clip, black where its shape and the frame's differ"
                        }
                    };
                    self.card_hint(d, state, widget, *chip, hint);
                    cx.controls
                        .push(format!("fit-{}@{}", fit.name(), rect_text(*chip)));
                    if let Some(glyph) = glyph {
                        cx.controls
                            .push(format!("fit-{}-glyph@{}", fit.name(), rect_text(glyph)));
                    }
                    if state.clicked && section.placement.fit != *fit {
                        edit(
                            cx,
                            ClipPlacement {
                                fit: *fit,
                                ..section.placement
                            },
                        );
                    }
                }
            }
        }
    }

    /// [`Self::kind_chip`]'s box, states and layout for any glyph: the glyph
    /// left of the name, dropped (never the name) where both do not fit.
    #[allow(
        clippy::too_many_arguments,
        reason = "card_chip's arguments plus the glyph it draws"
    )]
    fn glyph_chip(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        id: u64,
        rect: UiRect,
        label: &str,
        selected: bool,
        enabled: bool,
        glyph: impl FnOnce(
            &mut RaylibDrawHandle<'_>,
            UiRect,
            raylib::prelude::Color,
            raylib::prelude::Color,
        ),
    ) -> (widgets::ButtonState, Option<UiRect>) {
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
        let text = widgets::measure(font, label, metric::UI_FONT_CAPTION);
        let whole = look_cards::KIND_GLYPH + look_cards::KIND_GLYPH_GAP + text;
        let place = if whole + 8.0 <= rect.width {
            let left = (rect.x + (rect.width - whole) * 0.5).round();
            let top = (rect.y + (rect.height - look_cards::KIND_GLYPH) * 0.5).round();
            let place = UiRect::new(left, top, look_cards::KIND_GLYPH, look_cards::KIND_GLYPH);
            // What is not the choice itself (the clip playing, the frame
            // around it) is the same ink, half there: the rule tone the kinds
            // use vanishes on a chosen chip at this size.
            glyph(
                d,
                place,
                ink,
                widgets::alpha(ink, if enabled { 0.45 } else { 0.25 }),
            );
            widgets::draw_text(
                d,
                font,
                label,
                left + look_cards::KIND_GLYPH + look_cards::KIND_GLYPH_GAP,
                rect.y + (rect.height - metric::UI_FONT_CAPTION) * 0.5,
                metric::UI_FONT_CAPTION,
                ink,
            );
            Some(place)
        } else {
            widgets::text_label(d, font, rect, label, metric::UI_FONT_CAPTION, ink);
            None
        };
        (
            widgets::ButtonState {
                clicked: enabled && state.clicked,
                ..state
            },
            place,
        )
    }
}

/// The poster in its box, fitted by its aspect and centred.
fn draw_poster(
    d: &mut RaylibDrawHandle<'_>,
    poster: &raylib::prelude::RenderTexture2D,
    aspect: f32,
    well: UiRect,
    opacity: f32,
) {
    let inner = UiRect::new(
        well.x + 1.0,
        well.y + 1.0,
        well.width - 2.0,
        well.height - 2.0,
    );
    let aspect = aspect.max(0.1);
    let (width, height) = if inner.width / inner.height > aspect {
        (inner.height * aspect, inner.height)
    } else {
        (inner.width, inner.width / aspect)
    };
    let texture: &raylib::ffi::Texture2D = poster.as_ref();
    d.draw_texture_pro(
        poster,
        raylib::prelude::Rectangle::new(0.0, 0.0, texture.width as f32, texture.height as f32),
        raylib::prelude::Rectangle::new(
            (inner.x + (inner.width - width) * 0.5).round(),
            (inner.y + (inner.height - height) * 0.5).round(),
            width.round(),
            height.round(),
        ),
        raylib::prelude::Vector2::zero(),
        0.0,
        widgets::alpha(raylib::prelude::Color::WHITE, opacity),
    );
}

/// What happens after a clip's end, as clip time over section time, from
/// geometry alone: the clip playing in `dim`, what comes after it in `ink`.
fn draw_after_glyph(
    d: &mut RaylibDrawHandle<'_>,
    after: ClipOverrun,
    place: UiRect,
    ink: raylib::prelude::Color,
    dim: raylib::prelude::Color,
) {
    use raylib::prelude::Vector2;
    let (x, y, s) = (place.x, place.y, place.width);
    let at = |fx: f32, fy: f32| Vector2::new(x + fx * s, y + fy * s);
    // The clip plays: up to its end at the middle.
    d.draw_line_ex(at(0.0, 0.95), at(0.5, 0.1), 2.0, dim);
    match after {
        // The last frame stays: flat.
        ClipOverrun::Hold => {
            d.draw_line_ex(at(0.5, 0.1), at(1.0, 0.1), 2.0, ink);
            d.draw_circle_v(at(1.0, 0.1), s * 0.1, ink);
        }
        // From the top again: down at once, up again.
        ClipOverrun::Loop => {
            d.draw_line_ex(at(0.5, 0.1), at(0.5, 0.95), 1.0, dim);
            d.draw_line_ex(at(0.5, 0.95), at(1.0, 0.1), 2.0, ink);
        }
        // Back the way it came: a peak.
        ClipOverrun::Bounce => {
            d.draw_line_ex(at(0.5, 0.1), at(1.0, 0.95), 2.0, ink);
        }
    }
}

/// How a clip fits the frame, from geometry alone: the frame in `dim`, the
/// clip in `ink`.
fn draw_fit_glyph(
    d: &mut RaylibDrawHandle<'_>,
    fit: ClipFit,
    place: UiRect,
    ink: raylib::prelude::Color,
    dim: raylib::prelude::Color,
) {
    let (x, y, s) = (place.x, place.y, place.width);
    let block = |d: &mut RaylibDrawHandle<'_>, fx: f32, fy: f32, fw: f32, fh: f32, tint| {
        widgets::fill(d, UiRect::new(x + fx * s, y + fy * s, fw * s, fh * s), tint);
    };
    match fit {
        // The clip fills the frame; what is cropped off its sides is dim.
        ClipFit::Cover => {
            block(d, 0.0, 0.17, 0.17, 0.66, dim);
            block(d, 0.83, 0.17, 0.17, 0.66, dim);
            block(d, 0.17, 0.17, 0.66, 0.66, ink);
        }
        // The whole clip inside the frame, bars above and below.
        ClipFit::Contain => {
            d.draw_rectangle_lines_ex(
                raylib::prelude::Rectangle::new(x, y + 0.08 * s, s, 0.84 * s),
                1.0,
                dim,
            );
            block(d, 0.0, 0.3, 1.0, 0.4, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_end_and_fit_has_a_name_and_words() {
        let names: std::collections::BTreeSet<_> = ClipOverrun::ALL
            .iter()
            .map(|after| after_label(*after))
            .collect();
        assert_eq!(names.len(), 3);
        let fits: std::collections::BTreeSet<_> =
            ClipFit::ALL.iter().map(|fit| fit_label(*fit)).collect();
        assert_eq!(fits.len(), 2);
        for after in ClipOverrun::ALL {
            assert!(!after_words(after).is_empty());
        }
    }
}
