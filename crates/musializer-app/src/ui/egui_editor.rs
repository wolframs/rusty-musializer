//! Toolkit-owned controls for the scene inspector and lyric draft editor.
//!
//! Rendering only emits the same commands/edits as the existing shell. The
//! composition root retains settings, undo, dirty marking and file ownership.

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Color32, Context, RichText, Stroke};
use musializer_core::project::lyrics::{validate_text, LyricCue};
use musializer_core::scene::settings::{self, SettingKind};
use musializer_core::scene::SceneId;

use super::panels::lyrics::LyricsEdit;
use super::shell::ShellCommand;
use super::theme::UiTheme;
use crate::workspace::{SaveState, Track};

#[derive(Default)]
pub struct EditorOutput {
    pub commands: Vec<ShellCommand>,
    pub lyric_edits: Vec<LyricsEdit>,
    pub theme: Option<UiTheme>,
    pub close_requested: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Tune,
    Lyrics,
}

#[derive(Clone)]
struct CueDraft {
    original: LyricCue,
    text: String,
    start: f64,
    end: f64,
}

impl CueDraft {
    fn new(cue: &LyricCue) -> Self {
        Self {
            original: cue.clone(),
            text: cue.text.clone(),
            start: cue.start_seconds,
            end: cue.end_seconds,
        }
    }

    fn dirty(&self) -> bool {
        self.text != self.original.text
            || self.start != self.original.start_seconds
            || self.end != self.original.end_seconds
    }

    fn conflict(&self, cue: &LyricCue) -> bool {
        cue.text != self.original.text
            || cue.start_seconds != self.original.start_seconds
            || cue.end_seconds != self.original.end_seconds
    }

    fn valid(&self, duration: f64) -> bool {
        !self.text.trim().is_empty()
            && validate_text(&self.text).is_ok()
            && self.start.is_finite()
            && self.end.is_finite()
            && self.start >= 0.0
            && self.end > self.start
            && self.end <= duration
    }

    fn edit(&self) -> LyricsEdit {
        LyricsEdit::Update {
            id: self.original.id,
            start_seconds: self.start,
            end_seconds: self.end,
            text: self.text.clone(),
        }
    }
}

#[derive(Default)]
pub struct EditorState {
    pub theme: UiTheme,
    tab: Tab,
    selected: BTreeMap<PathBuf, u64>,
    // Track identity, not the current slot, owns unfinished text. Navigating
    // away and back must never move a draft onto another song with the same id.
    drafts: BTreeMap<(PathBuf, u64), CueDraft>,
    last_applied: Option<(PathBuf, u64)>,
}

/// Apply the selected skin without changing the renderer or document styling.
pub fn apply_theme(ctx: &Context, theme: UiTheme) {
    let rgba = |value: u32| {
        let [r, g, b, a] = value.to_be_bytes();
        Color32::from_rgba_unmultiplied(r, g, b, a)
    };
    use super::theme::dark;
    let dark = theme == UiTheme::BlackAmber;
    let mut style = egui::Style::default();
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let accent = if dark {
        Color32::from_rgb(255, 184, 0)
    } else {
        Color32::from_rgb(0, 47, 167)
    };
    let ink = if dark {
        Color32::from_rgb(225, 226, 216)
    } else {
        Color32::from_rgb(20, 20, 20)
    };
    let base = if dark {
        Color32::from_rgb(11, 12, 10)
    } else {
        Color32::from_rgb(247, 247, 248)
    };
    let raised = if dark {
        rgba(dark::CONTROL)
    } else {
        Color32::WHITE
    };
    let rule = if dark {
        rgba(dark::RULE)
    } else {
        Color32::from_rgb(170, 170, 180)
    };
    visuals.override_text_color = Some(ink);
    visuals.panel_fill = base;
    visuals.window_fill = if dark { rgba(dark::TOOLTIP) } else { base };
    visuals.extreme_bg_color = raised;
    visuals.faint_bg_color = raised;
    visuals.selection.bg_fill = if dark {
        Color32::from_rgb(91, 67, 0)
    } else {
        Color32::from_rgb(210, 223, 255)
    };
    visuals.selection.stroke = Stroke::new(1.0, ink);
    visuals.hyperlink_color = accent;
    visuals.window_corner_radius = egui::CornerRadius::ZERO;
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.window_stroke = Stroke::new(1.0, rule);
    for widgets in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widgets.corner_radius = egui::CornerRadius::ZERO;
        widgets.bg_stroke = Stroke::new(1.0, rule);
        widgets.fg_stroke = Stroke::new(1.0, ink);
        widgets.bg_fill = raised;
        widgets.weak_bg_fill = raised;
    }
    visuals.widgets.inactive.bg_fill = raised;
    if dark {
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, rgba(dark::CONTROL_EDGE));
        visuals.widgets.noninteractive.bg_fill = rgba(dark::RAISED);
        visuals.widgets.noninteractive.weak_bg_fill = rgba(dark::RAISED);
        visuals.window_stroke = Stroke::new(1.0, rgba(dark::CONTROL_EDGE));
    }
    visuals.widgets.hovered.bg_fill = if dark {
        rgba(dark::HOVER)
    } else {
        Color32::from_rgb(229, 234, 246)
    };
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.active.bg_fill = visuals.selection.bg_fill;
    visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
    visuals.widgets.open.bg_fill = visuals.selection.bg_fill;
    visuals.widgets.open.weak_bg_fill = visuals.widgets.open.bg_fill;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 10.0);
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    let family = if dark {
        egui::FontFamily::Monospace
    } else {
        egui::FontFamily::Proportional
    };
    for (text_style, size) in [
        (egui::TextStyle::Heading, 20.0),
        (egui::TextStyle::Body, 15.0),
        (egui::TextStyle::Button, 15.0),
        (egui::TextStyle::Small, 12.0),
    ] {
        style
            .text_styles
            .insert(text_style, egui::FontId::new(size, family.clone()));
    }
    let selected = if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(selected);
    ctx.set_style_of(selected, style);
}

fn theme_picker(ui: &mut egui::Ui, id: &str, theme: &mut UiTheme) -> bool {
    let previous = *theme;
    egui::ComboBox::from_id_salt(id)
        .selected_text(theme.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(theme, UiTheme::LightBlue, "Light blue");
            ui.selectable_value(theme, UiTheme::BlackAmber, "Black amber");
        });
    *theme != previous
}

/// The same appearance control is available before any project is opened.
pub fn welcome_theme(
    ctx: &Context,
    window: (f32, f32),
    theme: &mut UiTheme,
) -> (Option<UiTheme>, bool) {
    apply_theme(ctx, *theme);
    let mut changed = false;
    let popup_was_open = egui::Popup::is_any_open(ctx);
    let area = egui::Area::new(egui::Id::new("welcome_appearance"))
        .fixed_pos(egui::pos2((window.0 - 232.0).max(0.0), 24.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(200.0);
            ui.horizontal(|ui| {
                ui.label("Theme");
                changed = theme_picker(ui, "welcome_theme", theme);
            });
        });
    let over_picker = ctx
        .input(|input| input.pointer.interact_pos())
        .is_some_and(|pos| area.response.rect.contains(pos));
    (
        changed.then_some(*theme),
        over_picker || popup_was_open || egui::Popup::is_any_open(ctx),
    )
}

impl EditorState {
    /// Unapplied drafts remain unresolved work even when the editor is closed.
    pub fn has_dirty_drafts(&self) -> bool {
        self.drafts.values().any(CueDraft::dirty)
    }

    /// Select a cue already owned by this track without changing editor tabs.
    /// Returns false for stale ids so the caller can retain its current focus.
    pub fn focus_cue(&mut self, track: &Track, id: u64) -> bool {
        if track.lyrics.find(id).is_none() {
            return false;
        }
        self.selected.insert(track.file_path.clone(), id);
        true
    }

    /// The editor's current valid cue selection for this track.
    pub fn selected_cue(&self, track: &Track) -> Option<u64> {
        self.selected
            .get(&track.file_path)
            .copied()
            .filter(|id| track.lyrics.find(*id).is_some())
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        rect: egui::Rect,
        track: Option<&Track>,
        scene: SceneId,
        _track_slot: Option<usize>,
        playhead: f64,
        playing: bool,
    ) -> EditorOutput {
        apply_theme(ctx, self.theme);
        let mut output = EditorOutput::default();
        if ctx
            .input(|input| input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::F8))
        {
            output.close_requested = true;
        }
        egui::Area::new(egui::Id::new("musializer_editor"))
            .fixed_pos(rect.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(rect.width());
                ui.set_height(rect.height());
                ui.set_clip_rect(rect);
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .stroke(ui.visuals().window_stroke)
                    .inner_margin(14.0)
                    .show(ui, |ui| {
                        ui.set_min_size(egui::vec2(
                            (rect.width() - 28.0).max(0.0),
                            (rect.height() - 28.0).max(0.0),
                        ));
                        ui.horizontal(|ui| {
                            ui.heading("Editor");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    output.close_requested |= ui
                                        .button("Close")
                                        .on_hover_text("Close editor (Esc or F8). Unapplied lyric drafts are kept.")
                                        .clicked();
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.tab, Tab::Tune, "Tune");
                            ui.selectable_value(&mut self.tab, Tab::Lyrics, "Lyrics");
                            if ui
                                .add_enabled(track.is_some(), egui::Button::new(if playing { "Pause" } else { "Play" }))
                                .on_hover_text(if playing { "Pause the current track" } else { "Play the current track" })
                                .clicked()
                            {
                                output.commands.push(ShellCommand::TogglePlay);
                            }
                            let seekable = track.is_some_and(|track| track.transport_seekable);
                            if ui
                                .add_enabled(seekable, egui::Button::new("-5 s"))
                                .on_hover_text("Seek back 5 seconds")
                                .clicked()
                            {
                                output
                                    .commands
                                    .push(ShellCommand::Seek((playhead - 5.0).max(0.0)));
                            }
                            if ui
                                .add_enabled(seekable, egui::Button::new("+5 s"))
                                .on_hover_text("Seek forward 5 seconds")
                                .clicked()
                            {
                                let duration = track.map_or(0.0, |track| track.duration_seconds);
                                output
                                    .commands
                                    .push(ShellCommand::Seek((playhead + 5.0).min(duration)));
                            }
                        });
                        if theme_picker(ui, "editor_theme", &mut self.theme) {
                            output.theme = Some(self.theme);
                        }
                        ui.separator();
                        if let Some(track) = track {
                            let name = track.display_name();
                            ui.add(egui::Label::new(RichText::new(name).strong()).truncate())
                                .on_hover_text(name);
                            let save_state = track.save_state();
                            let save_enabled = save_state != SaveState::Saved;
                            if save_enabled
                                && ctx.input_mut(|input| {
                                    input.consume_shortcut(&egui::KeyboardShortcut::new(
                                        egui::Modifiers::COMMAND,
                                        egui::Key::S,
                                    ))
                                })
                            {
                                output.commands.push(ShellCommand::SaveProject);
                            }
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{playhead:.3} / {:.3} s",
                                        track.duration_seconds
                                    ))
                                    .monospace(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(save_enabled, egui::Button::new("Save project"))
                                            .on_hover_text(match save_state {
                                                SaveState::Saved => "The project is already saved",
                                                SaveState::NoProjectFile => "Choose a project file and save",
                                                _ => "Save current project changes",
                                            })
                                            .clicked()
                                        {
                                            output.commands.push(ShellCommand::SaveProject);
                                        }
                                        ui.label(RichText::new(save_state.label()).small());
                                    },
                                );
                            });
                            let body_height = ui.available_height().max(50.0);
                            egui::ScrollArea::vertical()
                                .id_salt("editor_body")
                                .max_height(body_height)
                                .show(ui, |ui| match self.tab {
                                    Tab::Tune => self.tune(ui, track, scene, &mut output),
                                    Tab::Lyrics => self.lyrics(ui, track, playhead, &mut output),
                                });
                        } else {
                            ui.label("Open a track to tune its scene or edit lyrics.");
                            if ui.button("Add audio").clicked() {
                                output.commands.push(ShellCommand::OpenAudio);
                            }
                        }
                    });
            });
        output
    }

    fn tune(&self, ui: &mut egui::Ui, track: &Track, scene: SceneId, output: &mut EditorOutput) {
        ui.heading(scene.display_name());
        ui.label(RichText::new("Changes update the preview immediately.").small());
        ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(80.0);
        let settings = track.effective_settings();
        for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
            ui.push_id(descriptor.key, |ui| {
                let mut value = settings.get(scene, index);
                let changed = match descriptor.kind {
                    SettingKind::Toggle => {
                        let mut checked = value >= 0.5;
                        let changed = ui.checkbox(&mut checked, descriptor.label).changed();
                        value = if checked { 1.0 } else { 0.0 };
                        changed
                    }
                    SettingKind::Slider => {
                        ui.label(descriptor.label);
                        ui.add(
                            egui::Slider::new(&mut value, descriptor.minimum..=descriptor.maximum)
                                .fixed_decimals(descriptor.precision as usize),
                        )
                        .changed()
                    }
                };
                if changed {
                    output.commands.push(ShellCommand::SetSetting {
                        scene,
                        index,
                        value,
                    });
                }
            });
        }
        ui.separator();
        if ui.button("Reset scene settings").clicked() {
            output.commands.push(ShellCommand::ResetScene(scene));
        }
        ui.label(RichText::new("Presets, Explore and audio mappings remain in Tune.").small());
    }

    fn lyrics(
        &mut self,
        ui: &mut egui::Ui,
        track: &Track,
        playhead: f64,
        output: &mut EditorOutput,
    ) {
        if track.lyrics.is_empty() {
            ui.label("This track has no lyric cues yet. Create them with Lyrics Assist or the lyric timeline.");
            return;
        }
        let selected = self
            .selected
            .entry(track.file_path.clone())
            .or_insert(track.lyrics.cues()[0].id);
        if track.lyrics.find(*selected).is_none() {
            *selected = track.lyrics.cues()[0].id;
        }
        let selected_text = track.lyrics.find(*selected).map_or(String::new(), |cue| {
            format!("{:.3}s  {}", cue.start_seconds, cue.text)
        });
        let cue_index = track
            .lyrics
            .cues()
            .iter()
            .position(|cue| cue.id == *selected)
            .unwrap_or(0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(cue_index > 0, egui::Button::new("Previous"))
                .clicked()
            {
                *selected = track.lyrics.cues()[cue_index - 1].id;
            }
            if ui
                .add_enabled(
                    cue_index + 1 < track.lyrics.len(),
                    egui::Button::new("Next"),
                )
                .clicked()
            {
                *selected = track.lyrics.cues()[cue_index + 1].id;
            }
            ui.label(
                RichText::new(format!("Cue {} of {}", cue_index + 1, track.lyrics.len())).small(),
            );
        });
        egui::ComboBox::from_id_salt("lyric_cue")
            .width(ui.available_width())
            .truncate()
            .selected_text(selected_text)
            .show_ui(ui, |ui| {
                for (index, cue) in track.lyrics.cues().iter().enumerate() {
                    ui.selectable_value(
                        selected,
                        cue.id,
                        format!(
                            "{}  ·  {:.3}s  ·  {}",
                            index + 1,
                            cue.start_seconds,
                            cue.text
                        ),
                    );
                }
            });
        let cue = track
            .lyrics
            .find(*selected)
            .expect("selected cue is in the document");
        let draft = self
            .drafts
            .entry((track.file_path.clone(), cue.id))
            .or_insert_with(|| CueDraft::new(cue));
        if !draft.dirty() && draft.conflict(cue) {
            *draft = CueDraft::new(cue);
        }
        ui.label("Lyric text");
        ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(f32::INFINITY));
        egui::Grid::new("lyric_edges")
            .num_columns(3)
            .show(ui, |ui| {
                ui.label("Start (seconds)");
                ui.add(
                    egui::DragValue::new(&mut draft.start)
                        .speed(0.01)
                        .fixed_decimals(3),
                );
                if ui
                    .button("At playhead")
                    .on_hover_text("Set the start to the current playback position")
                    .clicked()
                {
                    draft.start = playhead;
                }
                ui.end_row();
                ui.label("End (seconds)");
                ui.add(
                    egui::DragValue::new(&mut draft.end)
                        .speed(0.01)
                        .fixed_decimals(3),
                );
                if ui
                    .button("At playhead")
                    .on_hover_text("Set the end to the current playback position")
                    .clicked()
                {
                    draft.end = playhead;
                }
                ui.end_row();
            });
        let conflict = draft.conflict(cue);
        let valid = draft.valid(track.duration_seconds);
        let dirty = draft.dirty();
        if dirty {
            self.last_applied = None;
            ui.label(RichText::new("Unapplied changes").strong());
        } else if self.last_applied.as_ref() == Some(&(track.file_path.clone(), cue.id)) {
            ui.label(RichText::new("Applied to project — save to keep this change.").strong());
        } else {
            ui.label(RichText::new("Cue matches the project.").small());
        }
        if conflict {
            ui.label(
                "This cue changed elsewhere. Revert to load its latest version before applying.",
            );
        }
        if !valid {
            ui.label("Enter a single lyric line and a positive span within the track duration.");
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(dirty && valid && !conflict, egui::Button::new("Apply cue"))
                .clicked()
            {
                output.lyric_edits.push(draft.edit());
                self.last_applied = Some((track.file_path.clone(), cue.id));
                // Keep the old base until the root commits. Next frame a clean
                // draft refreshes from the authoritative document.
                *draft = CueDraft::new(cue);
            }
            if ui
                .add_enabled(dirty || conflict, egui::Button::new("Revert"))
                .clicked()
            {
                *draft = CueDraft::new(cue);
                self.last_applied = None;
            }
            if ui.button("Seek to cue").clicked() {
                output.commands.push(ShellCommand::Seek(cue.start_seconds));
            }
        });
        ui.label(RichText::new("Close Editor and use Ctrl+Z to undo applied changes. Closing keeps unapplied changes.").small());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::project::lyrics::CueOrigin;

    fn cue() -> LyricCue {
        LyricCue {
            id: 9,
            start_seconds: 1.0,
            end_seconds: 2.0,
            text: "A phrase".into(),
            origin: CueOrigin::UserApplied,
        }
    }

    #[test]
    fn welcome_picker_only_owns_its_header_not_the_open_audio_button() {
        let ctx = Context::default();
        let mut theme = UiTheme::LightBlue;
        for (point, owns) in [
            (egui::pos2(835.0, 40.0), true),
            (egui::pos2(184.0, 308.0), false),
        ] {
            // Match the real low-level backend; run_ui creates a background UI
            // and would conceal the whole-viewport pointer-ownership regression.
            for frame in 0..2 {
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 640.0),
                    )),
                    events: vec![egui::Event::PointerMoved(point)],
                    ..Default::default()
                });
                let (change, blocked) = welcome_theme(&ctx, (960.0, 640.0), &mut theme);
                let _ = ctx.end_pass();
                assert!(change.is_none());
                // Areas measure themselves in an invisible first pass.
                if frame > 0 {
                    assert_eq!(blocked, owns, "pointer at {point:?}");
                }
            }
        }
    }

    #[test]
    fn draft_refuses_bad_edges_and_detects_external_edits() {
        let cue = cue();
        let mut draft = CueDraft::new(&cue);
        draft.end = f64::NAN;
        assert!(!draft.valid(10.0));
        draft.end = 1.0;
        assert!(!draft.valid(10.0));
        draft.end = 11.0;
        assert!(!draft.valid(10.0));
        draft.end = 2.5;
        assert!(draft.valid(10.0));
        let mut changed = cue.clone();
        changed.text = "Edited elsewhere".into();
        assert!(draft.conflict(&changed));
        match draft.edit() {
            LyricsEdit::Update {
                id,
                end_seconds,
                text,
                ..
            } => {
                assert_eq!(id, 9);
                assert_eq!(end_seconds, 2.5);
                assert_eq!(text, "A phrase");
            }
            _ => panic!("draft must emit an update"),
        }
    }

    #[test]
    fn cue_focus_bridge_rejects_stale_ids_and_stays_track_scoped() {
        let mut editor = EditorState::default();
        let mut first =
            Track::new(PathBuf::from("/test/first.wav"), 10.0, SceneId::Spectrum, 7).unwrap();
        first.lyrics.insert(cue()).unwrap();
        let second = Track::new(
            PathBuf::from("/test/second.wav"),
            10.0,
            SceneId::Spectrum,
            7,
        )
        .unwrap();

        assert!(editor.focus_cue(&first, 9));
        assert_eq!(editor.selected_cue(&first), Some(9));
        assert_eq!(editor.selected_cue(&second), None);
        assert!(!editor.focus_cue(&first, 404));
        assert_eq!(editor.selected_cue(&first), Some(9));
        assert!(editor.tab == Tab::Tune);
    }

    #[test]
    fn both_themes_render_real_settings_and_lyrics_without_editing_on_redraw() {
        let ctx = Context::default();
        let mut track =
            Track::new(PathBuf::from("/test/song.wav"), 10.0, SceneId::Spectrum, 7).unwrap();
        track.lyrics.insert(cue()).unwrap();
        let mut editor = EditorState::default();
        for theme in [UiTheme::LightBlue, UiTheme::BlackAmber] {
            editor.theme = theme;
            for tab in [Tab::Tune, Tab::Lyrics] {
                editor.tab = tab;
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let result = editor.show(
                        ui.ctx(),
                        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(440.0, 600.0)),
                        Some(&track),
                        SceneId::Spectrum,
                        Some(0),
                        1.5,
                        false,
                    );
                    assert!(result.commands.is_empty());
                    assert!(result.lyric_edits.is_empty());
                    assert!(!result.close_requested);
                });
                assert!(!output.shapes.is_empty());
            }
        }
        assert_eq!(track.lyrics.find(9).unwrap().text, "A phrase");
    }

    fn draw(
        editor: &mut EditorState,
        ctx: &Context,
        track: &Track,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, EditorOutput) {
        let mut result = EditorOutput::default();
        let raw = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 800.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            result = editor.show(
                ui.ctx(),
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 750.0)),
                Some(track),
                SceneId::Spectrum,
                Some(0),
                1.5,
                false,
            );
        });
        (output, result)
    }

    fn text_point(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("label missing: {label}"))
    }

    fn click(
        editor: &mut EditorState,
        ctx: &Context,
        track: &Track,
        pos: egui::Pos2,
    ) -> EditorOutput {
        let event = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, mut pressed) = draw(
            editor,
            ctx,
            track,
            vec![egui::Event::PointerMoved(pos), event(true)],
        );
        let (_, released) = draw(editor, ctx, track, vec![event(false)]);
        pressed.commands.extend(released.commands);
        pressed.lyric_edits.extend(released.lyric_edits);
        pressed.close_requested |= released.close_requested;
        pressed
    }

    #[test]
    fn pointer_slider_changes_emit_real_setting_commands() {
        let ctx = Context::default();
        let mut editor = EditorState::default();
        let track =
            Track::new(PathBuf::from("/test/song.wav"), 10.0, SceneId::Spectrum, 7).unwrap();
        draw(&mut editor, &ctx, &track, vec![]);
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let first = &settings::descriptors(SceneId::Spectrum)[0];
        let label = text_point(&frame, first.label);
        let result = click(&mut editor, &ctx, &track, egui::pos2(250.0, label.y + 27.0));
        assert!(result.commands.iter().any(|command| matches!(command, ShellCommand::SetSetting { scene: SceneId::Spectrum, index: 0, value } if first.accepts(*value) && *value != first.default_value)));
    }

    #[test]
    fn compact_seek_buttons_bound_targets_and_disable_for_unseekable_tracks() {
        let ctx = Context::default();
        let mut editor = EditorState::default();
        let mut track =
            Track::new(PathBuf::from("/test/song.wav"), 4.0, SceneId::Spectrum, 7).unwrap();
        track.transport_seekable = true;
        draw(&mut editor, &ctx, &track, vec![]);

        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let back = click(&mut editor, &ctx, &track, text_point(&frame, "-5 s"));
        assert_eq!(back.commands, vec![ShellCommand::Seek(0.0)]);

        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let forward = click(&mut editor, &ctx, &track, text_point(&frame, "+5 s"));
        assert_eq!(forward.commands, vec![ShellCommand::Seek(4.0)]);

        track.transport_seekable = false;
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let disabled = click(&mut editor, &ctx, &track, text_point(&frame, "+5 s"));
        assert!(disabled.commands.is_empty());
    }

    #[test]
    fn save_button_and_close_shortcut_emit_existing_shell_actions() {
        let ctx = Context::default();
        let mut editor = EditorState::default();
        let mut track =
            Track::new(PathBuf::from("/test/song.wav"), 10.0, SceneId::Spectrum, 7).unwrap();
        track.mark_dirty(1.0);

        draw(&mut editor, &ctx, &track, vec![]);
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let saved = click(
            &mut editor,
            &ctx,
            &track,
            text_point(&frame, "Save project"),
        );
        assert_eq!(saved.commands, vec![ShellCommand::SaveProject]);

        let shortcut = draw(
            &mut editor,
            &ctx,
            &track,
            vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        )
        .1;
        assert_eq!(shortcut.commands, vec![ShellCommand::SaveProject]);

        let close = draw(
            &mut editor,
            &ctx,
            &track,
            vec![egui::Event::Key {
                key: egui::Key::F8,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .1;
        assert!(close.close_requested);
    }

    #[test]
    fn keyboard_text_edit_applies_through_existing_lyric_command_and_close_keeps_draft() {
        let ctx = Context::default();
        let mut editor = EditorState {
            tab: Tab::Lyrics,
            ..Default::default()
        };
        let mut track =
            Track::new(PathBuf::from("/test/song.wav"), 10.0, SceneId::Spectrum, 7).unwrap();
        track.lyrics.insert(cue()).unwrap();
        draw(&mut editor, &ctx, &track, vec![]);
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let label = text_point(&frame, "Lyric text");
        click(&mut editor, &ctx, &track, egui::pos2(150.0, label.y + 29.0));
        let select = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        draw(
            &mut editor,
            &ctx,
            &track,
            vec![select, egui::Event::Text("Changed phrase".into())],
        );
        draw(&mut editor, &ctx, &track, vec![]);
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let close = click(&mut editor, &ctx, &track, text_point(&frame, "Close"));
        assert!(close.close_requested);
        let draft = editor.drafts.get(&(track.file_path.clone(), 9)).unwrap();
        assert_eq!(draft.text, "Changed phrase");
        assert!(editor.has_dirty_drafts());
        draw(&mut editor, &ctx, &track, vec![]);
        let (frame, _) = draw(&mut editor, &ctx, &track, vec![]);
        let applied = click(&mut editor, &ctx, &track, text_point(&frame, "Apply cue"));
        assert!(!editor.has_dirty_drafts());
        assert!(
            matches!(applied.lyric_edits.as_slice(), [LyricsEdit::Update { id: 9, text, start_seconds: 1.0, end_seconds: 2.0 }] if text == "Changed phrase")
        );
    }

    #[test]
    fn both_skins_keep_body_selected_and_hovered_text_readable() {
        let packed = |color: Color32| u32::from_be_bytes([color.r(), color.g(), color.b(), 255]);
        let ctx = Context::default();
        for theme in [UiTheme::LightBlue, UiTheme::BlackAmber] {
            apply_theme(&ctx, theme);
            let style = ctx.style_of(if theme == UiTheme::BlackAmber {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let v = &style.visuals;
            for (name, ink, fill) in [
                ("body", v.text_color(), v.panel_fill),
                ("selection", v.selection.stroke.color, v.selection.bg_fill),
                (
                    "button",
                    v.widgets.inactive.fg_stroke.color,
                    v.widgets.inactive.weak_bg_fill,
                ),
                (
                    "active button",
                    v.widgets.active.fg_stroke.color,
                    v.widgets.active.weak_bg_fill,
                ),
                (
                    "hovered",
                    v.widgets.hovered.fg_stroke.color,
                    v.widgets.hovered.bg_fill,
                ),
            ] {
                let ratio = musializer_core::ui::contrast::ratio(packed(ink), packed(fill));
                assert!(ratio >= 4.5, "{theme:?} {name}: {ratio}");
            }
        }
    }
}
