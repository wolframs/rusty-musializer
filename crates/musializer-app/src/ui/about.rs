//! Attribution is ordinary chrome, with the notices embedded in core::credits.
//! A separate final draw keeps this surface above both raylib and egui.
use super::{
    shell::{Shell, ShellInput},
    theme::{color, metric},
    widgets::{self, ButtonStyle},
};
use musializer_core::{
    credits::{CREDITS, VERSION},
    ui::workspace_layout::UiRect,
};
use musializer_runtime::font::{AuthoredText, UiFonts};
use raylib::prelude::*;

#[derive(Default)]
pub struct About {
    pub open: bool,
    notice: Option<usize>,
    scroll: f32,
    pub(crate) logo: UiRect,
    welcome: UiRect,
    panel: UiRect,
    body: UiRect,
    close: UiRect,
    licence: UiRect,
    gap: UiRect,
    back: UiRect,
    down: UiRect,
    up: UiRect,
    overflow: f32,
    notice_key: Option<(usize, u32, u32, &'static str)>,
    notice_lines: Vec<String>,
    notice_face: &'static str,
    notice_missing: usize,
    last_action: &'static str,
}

fn layout(window: (f32, f32)) -> (UiRect, UiRect) {
    let width = (window.0 - 32.0).clamp(0.0, 760.0);
    let height = (window.1 - 32.0).clamp(0.0, 620.0);
    let panel = UiRect::new(
        (window.0 - width) * 0.5,
        (window.1 - height) * 0.5,
        width,
        height,
    );
    let body = UiRect::new(
        panel.x + 20.0,
        panel.y + 112.0,
        (width - 48.0).max(0.0),
        (height - 160.0).max(0.0),
    );
    (panel, body)
}

/// Split long URLs as well as prose; never ellipsize a licence notice.
fn wrap(font: &impl widgets::TextFace, text: &str, width: f32, size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines().map(|line| line.trim_start_matches('\u{feff}')) {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() {
                let candidate = format!("{line} {word}");
                if font.measure_text(&candidate, size, 0.0).x > width {
                    lines.push(std::mem::take(&mut line));
                } else {
                    line.push(' ');
                }
            }
            for ch in word.chars() {
                let candidate = format!("{line}{ch}");
                if !line.is_empty() && font.measure_text(&candidate, size, 0.0).x > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        lines.push(line);
    }
    lines
}

fn text(
    d: &mut RaylibDrawHandle<'_>,
    font: &UiFonts,
    lines: &[String],
    x: f32,
    y: &mut f32,
    size: f32,
    ink: Color,
) {
    for line in lines {
        widgets::draw_text(d, font, line, x, *y, size, ink);
        *y += size + 5.0;
    }
}

impl Shell {
    pub(crate) fn about_logo(&mut self, d: &mut RaylibDrawHandle<'_>, rect: UiRect) {
        self.about.logo = rect;
        let id = widgets::widget_id(widgets::id::ABOUT, 0);
        let state = self.widgets.button(d, id, rect);
        self.widgets.hint(d, state, id, rect, "About Musializer");
        if state.clicked {
            self.open_about();
        }
    }

    pub(crate) fn about_welcome(&mut self, d: &mut RaylibDrawHandle<'_>, input: &ShellInput<'_>) {
        let rect = UiRect::new(200.0, 28.0, 84.0, 28.0);
        self.about.welcome = rect;
        if self
            .widgets
            .text_button(
                d,
                input.fonts.ui(),
                widgets::widget_id(widgets::id::ABOUT, 1),
                rect,
                "About",
                false,
                ButtonStyle::Ghost,
                None,
            )
            .clicked
        {
            self.open_about();
        }
    }

    pub fn open_about(&mut self) {
        self.cancel_artist_edit();
        self.about.open = true;
        self.about.notice = None;
        self.about.scroll = 0.0;
        self.about.last_action = "opened";
        self.about.notice_key = None;
    }

    /// Called after egui, on the default framebuffer in logical coordinates.
    pub fn draw_about(&mut self, d: &mut RaylibDrawHandle<'_>, input: &ShellInput<'_>) {
        if !self.about.open {
            return;
        }
        // Do not leave typed characters queued for a lyric field after closing.
        while d.get_char_pressed().is_some() {}
        let escape =
            std::mem::take(&mut self.probe_escape) || d.is_key_pressed(KeyboardKey::KEY_ESCAPE);
        if escape {
            self.about.open = false;
            self.about.last_action = "escape";
            return;
        }
        let (panel, body) = layout(input.window);
        self.about.panel = panel;
        self.about.body = body;
        self.about.close = UiRect::new(panel.x + panel.width - 44.0, panel.y + 12.0, 28.0, 28.0);
        self.about.back = UiRect::new(panel.x + 20.0, panel.y + 76.0, 72.0, 26.0);
        self.about.up = UiRect::new(
            panel.x + panel.width - 108.0,
            panel.y + panel.height - 38.0,
            36.0,
            26.0,
        );
        self.about.down = UiRect::new(
            panel.x + panel.width - 64.0,
            panel.y + panel.height - 38.0,
            36.0,
            26.0,
        );
        self.about.gap = UiRect::new(panel.x + 8.0, panel.y + panel.height - 42.0, 6.0, 20.0);
        widgets::fill(
            d,
            UiRect::new(0.0, 0.0, input.window.0, input.window.1),
            color::scrim(),
        );
        widgets::floating_surface(d, panel);
        let font = input.fonts.ui();
        widgets::draw_text(
            d,
            font,
            &format!("Rusty Musializer {VERSION}"),
            panel.x + 20.0,
            panel.y + 18.0,
            22.0,
            color::ui_ink(),
        );
        widgets::draw_text(
            d,
            font,
            "A music-video tool built with Rust, raylib and egui.",
            panel.x + 20.0,
            panel.y + 50.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        let button =
            |bank: &mut widgets::Widgets, d: &mut RaylibDrawHandle<'_>, index, rect, label| {
                bank.text_button(
                    d,
                    font,
                    widgets::widget_id(widgets::id::ABOUT, index),
                    rect,
                    label,
                    false,
                    ButtonStyle::Ghost,
                    None,
                )
                .clicked
            };
        if button(&mut self.widgets, d, 2, self.about.close, "×") {
            self.about.open = false;
            self.about.last_action = "close";
        }
        let notice = self.about.notice;
        if notice.is_none() {
            self.about.notice_key = None;
            self.about.notice_face = "none";
            self.about.notice_missing = 0;
        }
        if notice.is_some() && button(&mut self.widgets, d, 3, self.about.back, "← Back") {
            self.about.notice = None;
            self.about.scroll = 0.0;
            self.about.last_action = "back";
        }
        if notice.is_none() {
            widgets::draw_micro(
                d,
                font,
                panel.x + 20.0,
                panel.y + 83.0,
                &[widgets::MicroRun::muted("CREDITS AND LICENCES")],
            );
        }
        let pointer = self.widgets.pointer(d);
        if body.contains_point(pointer.x, pointer.y) {
            let wheel = self.wheel_delta(d);
            self.about.scroll = (self.about.scroll - wheel * 60.0).clamp(0.0, self.about.overflow);
        }
        if button(&mut self.widgets, d, 4, self.about.up, "↑") {
            self.about.scroll = (self.about.scroll - body.height * 0.8).max(0.0);
        }
        if button(&mut self.widgets, d, 5, self.about.down, "↓") {
            self.about.scroll = (self.about.scroll + body.height * 0.8).min(self.about.overflow);
        }
        let old_clip = self.widgets.set_input_clip(Some(body));
        let mut clip = widgets::begin_scissor(d, body, input.ui_scale);
        let mut y = body.y - self.about.scroll;
        self.about.licence = UiRect::default();
        if let Some(index) = notice {
            let credit = &CREDITS[index];
            let heading = wrap(font, credit.name, body.width - 12.0, metric::UI_FONT_HEADER);
            text(
                &mut clip,
                font,
                &heading,
                body.x,
                &mut y,
                metric::UI_FONT_HEADER,
                color::ui_ink(),
            );
            y += 12.0;
            let chrome = AuthoredText::from_ui(font);
            let authored = input.fonts.authored();
            let key = (
                index,
                body.width.to_bits(),
                input.ui_scale.value().to_bits(),
                authored.name(),
            );
            if self.about.notice_key != Some(key) {
                let content = credit.notice.unwrap_or("");
                let missing = |face: AuthoredText<'_>| {
                    content
                        .chars()
                        .filter(|&ch| ch != '\u{feff}' && !face.repertoire().covers(ch))
                        .count()
                };
                let face = if missing(chrome) == 0 {
                    chrome
                } else {
                    authored
                };
                self.about.notice_face = face.name();
                self.about.notice_missing = missing(face);
                self.about.notice_lines =
                    wrap(&face, content, body.width - 12.0, metric::UI_FONT_CAPTION);
                self.about.notice_key = Some(key);
            }
            let face = if self.about.notice_face == chrome.name() {
                chrome
            } else {
                authored
            };
            let lines = &self.about.notice_lines;
            let step = metric::UI_FONT_CAPTION + 5.0;
            for (line_index, line) in lines.iter().enumerate() {
                let line_y = y + line_index as f32 * step;
                if line_y + step >= body.y && line_y <= body.y + body.height {
                    face.draw_text(
                        &mut clip,
                        line,
                        Vector2::new(body.x.round(), line_y.round()),
                        metric::UI_FONT_CAPTION,
                        0.0,
                        color::ui_ink(),
                    );
                }
            }
            y += lines.len() as f32 * step;
        } else {
            let mut group = "";
            for (index, credit) in CREDITS.iter().enumerate() {
                if credit.group != group {
                    if !group.is_empty() {
                        y += 12.0;
                    }
                    widgets::section_head(
                        &mut clip,
                        font,
                        UiRect::new(body.x, y, body.width - 12.0, 18.0),
                        &[widgets::MicroRun::muted(&credit.group.to_uppercase())],
                    );
                    y += 28.0;
                    group = credit.group;
                }
                let title_width = if credit.notice.is_some() {
                    body.width - 100.0
                } else {
                    body.width - 12.0
                };
                let title = wrap(font, credit.name, title_width, metric::UI_FONT_LABEL);
                let licence = UiRect::new(body.x + body.width - 88.0, y - 4.0, 76.0, 26.0);
                if credit.notice.is_some() {
                    if index == 0 {
                        self.about.licence = licence;
                    }
                    if button(
                        &mut self.widgets,
                        &mut clip,
                        100 + index as u32,
                        licence,
                        "Licence",
                    ) {
                        self.about.notice = Some(index);
                        self.about.scroll = 0.0;
                        self.about.last_action = "licence";
                    }
                }
                text(
                    &mut clip,
                    font,
                    &title,
                    body.x,
                    &mut y,
                    metric::UI_FONT_LABEL,
                    color::ui_ink(),
                );
                let author = wrap(
                    font,
                    credit.author,
                    body.width - 12.0,
                    metric::UI_FONT_CAPTION,
                );
                text(
                    &mut clip,
                    font,
                    &author,
                    body.x,
                    &mut y,
                    metric::UI_FONT_CAPTION,
                    color::ui_muted(),
                );
                if let Some(license) = credit.license {
                    let lines = wrap(font, license, body.width - 12.0, metric::UI_FONT_CAPTION);
                    text(
                        &mut clip,
                        font,
                        &lines,
                        body.x,
                        &mut y,
                        metric::UI_FONT_CAPTION,
                        color::ui_muted(),
                    );
                }
                for url in credit.urls {
                    let lines = wrap(font, url, body.width - 12.0, metric::UI_FONT_CAPTION);
                    text(
                        &mut clip,
                        font,
                        &lines,
                        body.x,
                        &mut y,
                        metric::UI_FONT_CAPTION,
                        color::ui_ink(),
                    );
                }
                y += 18.0;
            }
        }
        drop(clip);
        self.widgets.set_input_clip(old_clip);
        self.about.overflow = (y + self.about.scroll - body.y - body.height).max(0.0);
        self.about.scroll = self.about.scroll.min(self.about.overflow);
        if self.about.overflow > 0.0 {
            let track = UiRect::new(body.x + body.width - 4.0, body.y, 3.0, body.height);
            widgets::fill(d, track, color::ui_rule());
            let height =
                (body.height * body.height / (body.height + self.about.overflow)).max(18.0);
            let thumb = UiRect::new(
                track.x,
                track.y + (track.height - height) * self.about.scroll / self.about.overflow,
                track.width,
                height,
            );
            widgets::fill(d, thumb, color::ui_muted());
        }
        widgets::draw_text(
            d,
            font,
            "Scroll to read · URLs shown for reference",
            body.x,
            panel.y + panel.height - 30.0,
            metric::UI_FONT_MICRO,
            color::ui_muted(),
        );
        // A full-window control receives only presses OUTSIDE the panel. A gap
        // inside it is deliberately inert, rather than a second close button.
        if !panel.contains_point(pointer.x, pointer.y)
            && self
                .widgets
                .button(
                    d,
                    widgets::widget_id(widgets::id::ABOUT, 6),
                    UiRect::new(0.0, 0.0, input.window.0, input.window.1),
                )
                .clicked
        {
            self.about.open = false;
            self.about.last_action = "scrim";
        }
    }

    pub fn describe_about(&self) -> String {
        let a = &self.about;
        let at = |r: UiRect| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.width, r.height);
        format!("open={} rows={} notice={} action={} rect@{} body@{} logo@{} welcome@{} licence@{} close@{} gap@{} scrim@0,0,8,8 back@{} up@{} down@{} scroll={:.0}/{:.0} notice-face={} missing={}",
            a.open, CREDITS.len(), a.notice.map_or_else(|| "none".to_owned(), |n| n.to_string()), a.last_action,
            at(a.panel),at(a.body),at(a.logo),at(a.welcome),at(a.licence),at(a.close),at(a.gap),at(a.back),at(a.up),at(a.down),a.scroll,a.overflow,a.notice_face,a.notice_missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_notice_is_covered_by_the_builtin_legal_reader_repertoire() {
        use musializer_runtime::font::GlyphRepertoire;
        for credit in CREDITS {
            if let Some(notice) = credit.notice {
                let missing: Vec<_> = notice
                    .chars()
                    .filter(|&ch| ch != '\u{feff}' && !GlyphRepertoire::Curated.covers(ch))
                    .collect();
                assert_eq!(missing, Vec::<char>::new(), "{}", credit.name);
            }
        }
        let glfw = CREDITS
            .iter()
            .find(|c| c.name.starts_with("raylib component notices"))
            .unwrap()
            .notice
            .unwrap();
        assert!(
            GlyphRepertoire::InterfaceSubset.missing(glfw) > 0,
            "the supplementary Cyrillic contributors exercise the fallback"
        );
    }

    #[test]
    fn modal_layout_fits_both_window_sizes_and_scales() {
        for (width, height) in [(960.0, 640.0), (1280.0, 720.0)] {
            for scale in [1.0, 1.5] {
                let window = (width / scale, height / scale);
                let (panel, body) = layout(window);
                assert_eq!(panel.width, (window.0 - 32.0).min(760.0));
                assert_eq!(panel.height, (window.1 - 32.0).min(620.0));
                assert_eq!(body.height, panel.height - 160.0);
                assert!(body.height > 200.0);
                assert_eq!(panel.x, (window.0 - panel.width) * 0.5);
            }
        }
    }
}
