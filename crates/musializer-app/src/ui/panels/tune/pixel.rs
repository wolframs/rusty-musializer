//! Pixel look in the column: one filter over every scene, scoped to the video
//! or the section under the playhead. All writes cross the existing plan boundary.
use super::*;
use crate::ui::panels::scene_timeline::ScenePlanEdit;
use crate::ui::theme::UiTheme;
use musializer_core::project::pixel::{pixel_bounds, Pixel};
use musializer_core::ui::pixel_look::{self, PixelChoice};

impl Shell {
    pub(crate) fn probe_pixel_scope(&mut self, section: bool) {
        self.look_column.pixel_section = section;
        self.look_column.pixel_reveal = true;
    }

    pub(crate) fn describe_pixel_look(&self) -> String {
        self.look_column.pixel_report.clone()
    }

    pub(super) fn appearance_control(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        footer: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let area = pixel_look::appearance_rect(footer);
        let theme = crate::ui::theme::current();
        let picked = self.widgets.segmented(
            d,
            input.fonts.ui(),
            widgets::id::PIXEL_LOOK,
            50,
            area,
            &["Dark", "Light"],
            Some(usize::from(theme == UiTheme::LightBlue)),
            widgets::SegmentedFit::Fill,
        );
        if let Some(index) = picked.clicked {
            commands.push(self.set_appearance(if index == 0 {
                UiTheme::BlackAmber
            } else {
                UiTheme::LightBlue
            }));
        }
        self.look_column.appearance_report = picked
            .layout
            .items
            .iter()
            .enumerate()
            .map(|(i, r)| format!("theme-{i}@{}", rect(*r)))
            .collect::<Vec<_>>()
            .join(";");
    }

    pub(super) fn pixel_look_column(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        input: &ShellInput<'_>,
        area: UiRect,
        commands: &mut Vec<ShellCommand>,
    ) {
        let Some(track) = input.workspace.current() else {
            return;
        };
        let font = input.fonts.ui();
        let layout = pixel_look::layout(area);
        let plan = &track.scene_switches;
        let video = plan.finish.pixel;
        let covering = plan.cue_covering(input.time_seconds);
        if covering.is_none() {
            self.look_column.pixel_section = false;
        }
        let section = self.look_column.pixel_section;
        let current = if section {
            covering.and_then(|cue| cue.pixel)
        } else {
            Some(video)
        };
        let effective = current.unwrap_or(video);
        let shown = if section && current.is_none() {
            PixelChoice::Video
        } else {
            PixelChoice::of(&effective)
        };
        widgets::section_head(
            d,
            font,
            layout.head,
            &[widgets::MicroRun::muted("Pixel look")],
        );
        widgets::draw_text_faded(
            d,
            font,
            "A pixel screen over any scene.",
            layout.invitation.x,
            layout.invitation.y,
            layout.invitation.width,
            12.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        let mut controls = Vec::new();
        let scopes: &[&str] = if covering.is_some() {
            &["Whole video", "This section"]
        } else {
            &["Whole video"]
        };
        let picked = self.widgets.segmented(
            d,
            font,
            widgets::id::PIXEL_LOOK,
            0,
            layout.scope,
            scopes,
            Some(usize::from(section)),
            widgets::SegmentedFit::Fill,
        );
        for (i, r) in picked.layout.items.iter().enumerate() {
            controls.push(format!("scope-{i}@{}", rect(*r)));
        }
        if let Some(i) = picked.clicked {
            self.look_column.pixel_section = i == 1;
            self.look_column.pixel_typing = None;
        }
        let mut next = None;
        for (row, boundary) in layout.choices.iter().enumerate() {
            let choices: Vec<_> = if row == 3 {
                let mut choices = vec![PixelChoice::Bare];
                if section {
                    choices.push(PixelChoice::Video);
                }
                choices
            } else {
                PixelChoice::OFFERED[row * 2..row * 2 + 2].to_vec()
            };
            let labels: Vec<_> = choices.iter().map(|choice| choice.label()).collect();
            let picked = self.widgets.segmented(
                d,
                font,
                widgets::id::PIXEL_LOOK,
                10 + row as u32 * 2,
                *boundary,
                &labels,
                choices.iter().position(|choice| *choice == shown),
                widgets::SegmentedFit::Fill,
            );
            for (i, r) in picked.layout.items.iter().enumerate() {
                controls.push(format!("choice-{}@{}", row * 2 + i, rect(*r)));
            }
            if let Some(i) = picked.clicked {
                if choices[i] != shown {
                    next = Some(choices[i].choose(current, video));
                }
            }
        }
        let owned = current.filter(Pixel::is_on);
        if let Some(look) = owned {
            let (low, high) = pixel_bounds::PIXELS;
            let result = self.widgets.slider_row(
                d,
                font,
                widgets::id::PIXEL_LOOK,
                30,
                layout.size,
                &widgets::SliderRow {
                    label: "Pixels tall",
                    value_text: &look.pixels.to_string(),
                    normalized: f32::from(look.pixels - low) / f32::from(high - low),
                    default: Some(f32::from(180 - low) / f32::from(high - low)),
                    changed: look.pixels != 180,
                    route: None,
                    label_width: 90.0,
                },
            );
            controls.push(format!(
                "size@{};value@{}",
                rect(result.layout.bar),
                rect(result.layout.value)
            ));
            if let Some(fraction) = result.drag {
                next = Some(Some(Pixel {
                    pixels: (f32::from(low) + fraction * f32::from(high - low)).round() as u16,
                    ..look
                }));
            }
            if result.value.clicked {
                let mut field = TextField::new(TextRules::ascii_query(6));
                field.bind(&look.pixels.to_string());
                field.set_focused(true);
                self.look_column.pixel_typing = Some(field);
            }
            if let Some(field) = self.look_column.pixel_typing.as_mut() {
                let commit = d.is_key_pressed(KeyboardKey::KEY_ENTER)
                    || d.is_key_pressed(KeyboardKey::KEY_KP_ENTER);
                let cancel = d.is_key_pressed(KeyboardKey::KEY_ESCAPE);
                widgets::fill(d, result.layout.value, color::ui_surface());
                field.draw_with_face(
                    d,
                    AuthoredText::from_ui(font),
                    result.layout.value,
                    metric::UI_FONT_VALUE,
                    "",
                    true,
                );
                if commit {
                    if let Ok(pixels) = field.edit.text().parse::<u16>() {
                        next = Some(Some(Pixel {
                            pixels: pixels.clamp(low, high),
                            ..look
                        }));
                    }
                }
                if commit || cancel || !field.is_focused() {
                    self.look_column.pixel_typing = None;
                }
            }
            self.widgets.hint(d, result.track, widgets::widget_id(widgets::id::PIXEL_LOOK,30), result.layout.track,
                "Pixels across the short edge: 144 is a handheld, 180 a cartridge. Click the value to type.");
            let kicks = look.split > 0.0 || look.shake > 0.0 || look.zoom > 0.0;
            let picked = self.widgets.segmented(
                d,
                font,
                widgets::id::PIXEL_LOOK,
                40,
                layout.kick,
                &["Kick off", "Answer the kick"],
                Some(usize::from(kicks)),
                widgets::SegmentedFit::Fill,
            );
            for (i, r) in picked.layout.items.iter().enumerate() {
                controls.push(format!("kick-{i}@{}", rect(*r)));
            }
            if let Some(i) = picked.clicked {
                if (i == 1) != kicks {
                    next = Some(Some(pixel_look::kick(look, i == 1)));
                }
            }
        } else {
            self.look_column.pixel_typing = None;
        }
        let status = if shown == PixelChoice::Listed {
            "Listed colours".to_owned()
        } else if section {
            let index = covering
                .and_then(|cue| plan.cues().iter().position(|c| c.id == cue.id))
                .unwrap_or(0);
            format!(
                "Section {} of {} · {}",
                index + 1,
                plan.len(),
                shown.label()
            )
        } else if covering.is_none() {
            "Any scene · add a plan for section overrides".to_owned()
        } else {
            format!("Any scene · {}", shown.label())
        };
        widgets::draw_text_faded(
            d,
            font,
            &status,
            layout.status.x,
            layout.status.y,
            layout.status.width,
            12.0,
            metric::UI_FONT_MICRO,
            color::ui_muted(),
        );
        if let Some(pixel) = next {
            commands.push(ShellCommand::ScenePlan(if section {
                ScenePlanEdit::SetPixel {
                    cue_id: covering.unwrap().id,
                    pixel,
                }
            } else {
                ScenePlanEdit::SetVideoPixel(pixel.unwrap_or(Pixel::OFF))
            }));
        }
        self.look_column.pixel_report = format!("scope={} current={} pixels={} typing={} kick={:.3},{:.3},{:.3} video={} section={} area={} controls={} {}",
            if section {"section"} else {"video"}, shown.label().replace(' ',"-"), effective.pixels, self.look_column.pixel_typing.is_some(),
            effective.split, effective.shake, effective.zoom, pixel_state(Some(video)), pixel_state(covering.and_then(|cue|cue.pixel)),
            rect(area), controls.join(";"), self.look_column.appearance_report);
    }
}
fn rect(r: UiRect) -> String {
    format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height)
}
fn pixel_state(pixel: Option<Pixel>) -> String {
    match pixel {
        None => "inherit".to_owned(),
        Some(p) if !p.is_on() => "off".to_owned(),
        Some(p) => format!(
            "{}:{}:{}:{:.3},{:.3},{:.3}",
            PixelChoice::of(&p).label().replace(' ', "-"),
            p.pixels,
            p.matching.name(),
            p.split,
            p.shake,
            p.zoom
        ),
    }
}
