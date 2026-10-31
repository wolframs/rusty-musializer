//! A native chat with Claude Code, Codex or Antigravity in the dock (WX-4).
//!
//! The transcript and the three wire protocols are in
//! `musializer_core::agent`; the process is `musializer_runtime::agent_process`.
//! This file starts the right command line for each agent, pumps lines
//! between the two, and draws the conversation: what was said, one line per
//! tool call (expandable to its output), Allow/Deny on a permission prompt,
//! and a composer.
//!
//! **Context travels with a message, not as a hidden prompt.** The
//! "playhead" toggle prefixes what you send with one bracketed line naming the
//! playhead, the section under it and the followed treatment. That line is
//! shown in your bubble, so what the agent was told is on screen. The standing
//! instructions (what Musializer is, where the guide is, how to take a still
//! without opening a window) go once per session: Claude's
//! `--append-system-prompt`, Codex's thread `developerInstructions`, and the
//! first message for Antigravity, which takes neither.
//!
//! **The composer's three chips** pick the model, the effort and the access,
//! from what the CLI itself lists (`core::agent`). Each menu says when its
//! choice lands where that is not at once. A new chat starts on what this
//! agent last chose (`agent_choices`), except Full access.

use egui::text::{LayoutJob, TextWrapping};
use egui::{Key, Modifiers, RichText, Stroke};
use musializer_core::agent::{
    antigravity_models, effort_title, Access, Entry, Protocol, Selection, ToolState, Transcript,
};
use musializer_runtime::agent_process::{AgentProcess, Launch, Output};

use super::{Agent, DockContext};
use crate::ui::theme::color32;

/// The composer's height with its gap to the column's edge: the context row,
/// the words, the controls row.
const COMPOSER_HEIGHT: f32 = 132.0;
const CHIP_HEIGHT: f32 = 28.0;
const CHIP_GAP: f32 = 4.0;
const CHIP_PADDING: f32 = 8.0;
const GLYPH_WIDTH: f32 = 14.0;
const GLYPH_GAP: f32 = 5.0;
/// What the model chip may be cut down to before the access chip gives up
/// its words and keeps its glyph.
const MODEL_CHIP_LEAST: f32 = 76.0;
const MENU_WIDTH: f32 = 310.0;
const MODEL_MENU_HEIGHT: f32 = 320.0;

/// A choice made in one of the composer's menus.
enum Pick {
    Model(String),
    Effort(String),
    Access(Access),
}

/// `agy models`, running. Antigravity's protocol has no catalogue, so the
/// list comes from a process of its own, which the UI thread never waits on.
struct ModelListing {
    process: AgentProcess,
    text: String,
}

pub struct ChatTab {
    pub agent: Agent,
    transcript: Transcript,
    process: Option<AgentProcess>,
    composer: String,
    /// Prefix each message with the playhead line.
    with_context: bool,
    /// The chips changed since the choices were last saved.
    selection_changed: bool,
    listing: Option<ModelListing>,
    /// The standing instructions still owed to an agent that takes them with
    /// the first message.
    preamble_pending: bool,
    instructions: String,
    error: Option<String>,
    pub focus_next: bool,
    composer_focused: bool,
    id: egui::Id,
    /// Tool chips opened to show what they returned (the column's chat).
    expanded: std::collections::HashSet<usize>,
    note_times: std::collections::HashMap<usize, std::time::Instant>,
    /// What the column's chat drew in its last frame, kind and rect, for the
    /// `dock:` line (WX-5d): every probe and pixel check reads its rects here.
    pub layout: Vec<(&'static str, egui::Rect)>,
    /// The rows of the open menu, `model:ID`, `effort:LEVEL` or `access:KEY`.
    pub menu_rows: Vec<(String, egui::Rect)>,
}

/// What a frame of the chat asks for.
#[derive(Default)]
pub struct ChatOutput {
    pub has_focus: bool,
}

fn protocol(agent: Agent) -> Protocol {
    match agent {
        Agent::Claude => Protocol::Claude,
        Agent::Codex => Protocol::Codex,
        Agent::Antigravity => Protocol::Antigravity,
    }
}

impl ChatTab {
    pub fn new(agent: Agent, context: &DockContext, id: egui::Id, selection: Selection) -> Self {
        let mut transcript = Transcript::new(protocol(agent));
        transcript.selection = selection;
        let mut tab = Self {
            agent,
            transcript,
            process: None,
            composer: String::new(),
            with_context: true,
            selection_changed: false,
            listing: None,
            preamble_pending: agent == Agent::Antigravity,
            instructions: context.instructions.clone(),
            error: None,
            focus_next: true,
            composer_focused: false,
            id,
            expanded: std::collections::HashSet::new(),
            note_times: std::collections::HashMap::new(),
            layout: Vec::new(),
            menu_rows: Vec::new(),
        };
        tab.start(context, None);
        if agent == Agent::Antigravity {
            tab.list_models(context);
        }
        tab
    }

    /// The agent's program: its override, else its command on the shell's PATH.
    fn launch_with(&self, context: &DockContext, args: Vec<String>) -> Launch {
        let override_var = format!("MUSIALIZER_AGENT_{}", self.agent.command().to_uppercase());
        let overridden = std::env::var(&override_var).ok().filter(|v| !v.is_empty());
        Launch {
            program: overridden
                .clone()
                .unwrap_or_else(|| self.agent.command().to_string()),
            args,
            working_directory: context.working_directory.clone(),
            env: context.env.clone(),
            use_shell_path: overridden.is_none(),
        }
    }

    fn list_models(&mut self, context: &DockContext) {
        match AgentProcess::spawn(&self.launch_with(context, vec!["models".into()])) {
            Ok(process) => {
                self.listing = Some(ModelListing {
                    process,
                    text: String::new(),
                });
            }
            Err(error) => self.transcript.models_error = Some(error.to_string()),
        }
    }

    fn launch(&self, context: &DockContext, resume: Option<&str>) -> Launch {
        let args: Vec<String> = match self.agent {
            Agent::Claude => {
                let mut args: Vec<String> = [
                    "-p",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--include-partial-messages",
                    "--permission-prompt-tool",
                    "stdio",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
                args.extend(self.transcript.launch_flags());
                if !self.instructions.is_empty() {
                    args.push("--append-system-prompt".into());
                    args.push(self.instructions.clone());
                }
                if let Some(session) = resume {
                    args.push("--resume".into());
                    args.push(session.to_string());
                }
                args
            }
            Agent::Codex => vec!["app-server".into()],
            Agent::Antigravity => {
                let mut args: Vec<String> = [
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
                args.extend(self.transcript.launch_flags());
                if let Some(session) = resume {
                    args.push("--conversation".into());
                    args.push(session.to_string());
                }
                // Last, attached: agy's `-p` takes the next word as the prompt.
                args.push("--print=".into());
                args
            }
        };
        self.launch_with(context, args)
    }

    fn start(&mut self, context: &DockContext, resume: Option<&str>) {
        if let Some(process) = self.process.as_mut() {
            process.close();
        }
        match AgentProcess::spawn(&self.launch(context, resume)) {
            Ok(mut process) => {
                let cwd = context
                    .working_directory
                    .as_ref()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default();
                for line in self.transcript.start(&cwd, &self.instructions) {
                    let _ = process.write_line(&line);
                }
                self.process = Some(process);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.process = None;
            }
        }
    }

    /// Pumps the agent's output into the transcript; called every frame,
    /// visible or not, so a long turn keeps running while the dock is hidden.
    pub fn tick(&mut self) {
        self.tick_listing();
        self.selection_changed |= self.transcript.take_selection_reverted();
        let Some(process) = self.process.as_mut() else {
            return;
        };
        for output in process.poll() {
            match output {
                Output::Line(line) => {
                    for reply in self.transcript.receive(&line) {
                        let _ = process.write_line(&reply);
                    }
                }
                Output::Diagnostic(line) => self.transcript.diagnostic(&line),
                Output::Exited(how) => self.transcript.exited(&how),
            }
        }
    }

    fn tick_listing(&mut self) {
        let Some(listing) = self.listing.as_mut() else {
            return;
        };
        let mut done = false;
        for output in listing.process.poll() {
            match output {
                Output::Line(line) => {
                    listing.text.push_str(&line);
                    listing.text.push('\n');
                }
                Output::Diagnostic(_) => {}
                Output::Exited(_) => done = true,
            }
        }
        if !done {
            return;
        }
        let text = self.listing.take().map(|l| l.text).unwrap_or_default();
        let models = antigravity_models(&text);
        if models.is_empty() {
            self.transcript.models_error = Some("agy models listed nothing".to_owned());
        } else {
            let lines = self.transcript.set_models(models);
            self.write(lines);
        }
    }

    fn write(&mut self, lines: Vec<String>) {
        if let Some(process) = self.process.as_mut() {
            for line in lines {
                let _ = process.write_line(&line);
            }
        }
    }

    fn pick(&mut self, pick: Pick) {
        let lines = match pick {
            Pick::Model(id) => self.transcript.choose_model(&id),
            Pick::Effort(level) => self.transcript.choose_effort(&level),
            Pick::Access(access) => self.transcript.choose_access(access),
        };
        self.write(lines);
        self.selection_changed = true;
    }

    /// The selection to save, once per change.
    pub fn take_selection_change(&mut self) -> Option<Selection> {
        std::mem::take(&mut self.selection_changed).then(|| self.transcript.selection.clone())
    }

    pub fn close(&mut self) {
        if let Some(process) = self.process.as_mut() {
            process.close();
        }
        if let Some(mut listing) = self.listing.take() {
            listing.process.close();
        }
    }

    /// What the Direct tab's dot reads while the column shows another view
    /// (WX-5d §2): whether the agent works, waits on Allow/Deny, and how many
    /// replies it has written.
    #[must_use]
    pub fn activity(&self) -> (bool, bool, usize) {
        let (_, replies, _, _) = self.transcript.counts();
        (
            self.transcript.busy,
            self.transcript.awaiting_permission(),
            replies,
        )
    }

    #[must_use]
    pub fn title(&self) -> String {
        let marker = if self.transcript.awaiting_permission() {
            " ?"
        } else if self.transcript.busy {
            " …"
        } else {
            ""
        };
        format!("{}{marker}", self.agent.label())
    }

    fn send(&mut self, context: &DockContext) {
        let typed = self.composer.trim().to_string();
        if typed.is_empty() {
            return;
        }
        // A stopped agent is restarted, resuming its own session when it
        // reported one, so a crash or a Stop does not lose the conversation.
        // So is one that owes a choice to a new process, once it is idle.
        let owed = !self.transcript.busy && self.transcript.take_restart_owed();
        if owed || self.process.as_ref().is_none_or(AgentProcess::has_exited) {
            let resume = self.transcript.session.clone();
            self.start(context, resume.as_deref());
        }
        // Failed restarts must leave the draft and transcript untouched. The
        // start error is already shown above the composer.
        if self.process.is_none() {
            return;
        }
        let text = if self.with_context && !context.now.is_empty() {
            format!("[Musializer: {}]\n{typed}", context.now)
        } else {
            typed
        };
        let preamble = if self.preamble_pending {
            self.preamble_pending = false;
            Some(self.instructions.clone())
        } else {
            None
        };
        let lines = self.transcript.send(&text, preamble.as_deref());
        if let Some(process) = self.process.as_mut() {
            for line in lines {
                if let Err(error) = process.write_line(&line) {
                    self.error = Some(format!("could not reach the agent: {error}"));
                }
            }
        }
        self.composer.clear();
    }

    /// Answers every open permission prompt (`--ui-probe agent-decide=`).
    pub fn probe_decide(&mut self, allow: bool) {
        let open: Vec<usize> = self
            .transcript
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, Entry::Permission { allowed: None, .. }))
            .map(|(index, _)| index)
            .collect();
        for index in open {
            let lines = self.transcript.decide(index, allow);
            if let Some(process) = self.process.as_mut() {
                for line in lines {
                    let _ = process.write_line(&line);
                }
            }
        }
    }

    /// Sends `text` as if typed (`--ui-probe agent-say=`).
    pub fn probe_say(&mut self, text: &str, context: &DockContext) {
        self.composer = text.to_string();
        self.send(context);
    }

    /// One line for the run report.
    #[must_use]
    pub fn describe(&self) -> String {
        let (users, replies, tools, prompts) = self.transcript.counts();
        let notes: Vec<_> = self
            .transcript
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                Entry::Notice { text, .. } => Some(self.note_times.get(&index).map_or_else(
                    || text.clone(),
                    |at| format!("{text} · {}", note_age(at.elapsed().as_secs())),
                )),
                _ => None,
            })
            .collect();
        format!(
            "chat users={users} replies={replies} tools={tools} prompts={prompts} busy={} model={} last={:?}{} notes={notes:?} draft-lines={} pick=model:{}/effort:{}/access:{} chips=\"{}|{}|{}\" listed={} mode={}",
            self.transcript.busy,
            self.transcript.model.as_deref().unwrap_or("?"),
            self.transcript.last_reply().unwrap_or("").chars().take(120).collect::<String>(),
            self.error
                .as_deref()
                .map_or_else(String::new, |error| format!(" error={error:?}")),
            self.composer.lines().count(),
            self.transcript.selection.model.as_deref().unwrap_or("-"),
            self.transcript.selection.effort.as_deref().unwrap_or("-"),
            self.transcript.selection.access.key(),
            self.transcript.model_label(),
            self.transcript.effort_label(),
            self.transcript.selection.access.chip(),
            self.transcript.models.len(),
            self.transcript.reported_mode.as_deref().unwrap_or("-"),
        )
    }
}

pub(super) fn note_age(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s ago")
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else {
        format!("{} h ago", seconds / 3600)
    }
}

/// An assistant message: prose wraps; fenced code is monospace.
fn assistant_text(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut block = String::new();
    let flush = |ui: &mut egui::Ui, block: &mut String, code: bool| {
        let trimmed = block.trim_end_matches('\n');
        if !trimmed.is_empty() {
            let rich = if code {
                RichText::new(trimmed).monospace().small()
            } else {
                RichText::new(trimmed)
            };
            ui.add(egui::Label::new(rich).wrap());
        }
        block.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(ui, &mut block, in_code);
            in_code = !in_code;
            continue;
        }
        block.push_str(line);
        block.push('\n');
    }
    flush(ui, &mut block, in_code);
}

/// The colours the column's chat is drawn in (WX-5d §3.1), from the palette's
/// roles so both appearances read.
struct ChatInk {
    ink: egui::Color32,
    weak: egui::Color32,
    rule: egui::Color32,
    raised: egui::Color32,
    trough: egui::Color32,
    selected: egui::Color32,
    surface: egui::Color32,
    warning: egui::Color32,
    danger: egui::Color32,
    good: egui::Color32,
}

impl ChatInk {
    fn new(ui: &egui::Ui) -> Self {
        let palette = crate::ui::theme::palette();
        let of = crate::ui::theme::color32::of;
        Self {
            ink: color32::ui_ink(),
            weak: ui.visuals().weak_text_color(),
            rule: of(palette.ui_rule),
            raised: of(palette.ui_raised),
            trough: of(palette.ui_lane_trough),
            selected: of(palette.ui_selected),
            surface: of(palette.ui_surface),
            warning: color32::ui_warning(),
            danger: color32::ui_danger(),
            good: color32::ui_success(),
        }
    }
}

/// A micro head in egui: 11 px, tracked, uppercase.
fn micro(text: &str, colour: egui::Color32) -> RichText {
    RichText::new(text.to_uppercase())
        .size(crate::ui::theme::metric::UI_FONT_MICRO)
        .extra_letter_spacing(1.0)
        .color(colour)
}

/// A message's context line (`[Musializer: playhead 3:12.0 · …]`), split
/// from its words, so the bubble shows what was typed and the micro line
/// above it says where.
fn split_context(text: &str) -> (Option<&str>, &str) {
    if let Some(rest) = text.strip_prefix("[Musializer: ") {
        if let Some((context, words)) = rest.split_once("]\n") {
            return (Some(context), words);
        }
    }
    (None, text)
}

/// What a tool call is called on its chip: the verb the agent used.
fn tool_glyph(name: &str) -> &'static str {
    match name {
        "Bash" | "commandExecution" | "shell" => ">_",
        _ => "\u{00b7}",
    }
}

impl ChatTab {
    /// Appends a line the window says, not the agent: the treatment it
    /// applied (WX-5d §3.1, "the window's own applies appear in the
    /// transcript of the chat that is following").
    pub fn note(&mut self, text: String, applied_at: Option<std::time::Instant>) {
        if let Some(at) = applied_at {
            self.note_times.insert(self.transcript.entries.len(), at);
        }
        self.transcript
            .entries
            .push(Entry::Notice { text, error: false });
    }

    pub(super) fn needs_close_confirmation(&self) -> bool {
        self.transcript.busy || !self.composer.is_empty()
    }

    /// Whether the chat could not start.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }

    /// Whether the agent's process has ended (a Stop, a crash); sending
    /// starts it again.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.process.as_ref().is_none_or(AgentProcess::has_exited)
    }

    /// The chat as the column's Direct view draws it (WX-5d §3.1): the
    /// transcript — your bubbles right with their context line, the agent's
    /// prose left, tool calls as one-line chips, Allow/Deny as a
    /// warning-outlined card — over a rounded composer with the playhead
    /// chip and a round send button.
    pub fn show_column(&mut self, ui: &mut egui::Ui, context: &DockContext) -> ChatOutput {
        let mut output = ChatOutput::default();
        let c = ChatInk::new(ui);
        let mono = egui::FontId::monospace(11.5);

        // The composer is pinned at the bottom; measure it first.
        let composer_height = COMPOSER_HEIGHT;
        let full = ui.available_rect_before_wrap();
        let transcript_rect = egui::Rect::from_min_max(
            full.min,
            egui::pos2(
                full.right(),
                (full.bottom() - composer_height).max(full.top()),
            ),
        );
        let composer_rect = egui::Rect::from_min_max(
            egui::pos2(full.left() + 12.0, transcript_rect.bottom()),
            egui::pos2(full.right() - 12.0, full.bottom() - 12.0),
        );

        let mut decisions = Vec::new();
        let mut toggled = None;
        let mut layout: Vec<(&'static str, egui::Rect)> = Vec::new();
        let mut transcript_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(transcript_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::ScrollArea::vertical()
            .id_salt(self.id.with("column_scroll"))
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(&mut transcript_ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(14, 14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 12.0;
                        if self.transcript.entries.is_empty() {
                            self.first_words(ui, context, &c);
                        }
                        if let Some(error) = &self.error {
                            ui.label(
                                RichText::new(format!(
                                    "{} could not start: {error}",
                                    self.agent.label()
                                ))
                                .color(c.danger),
                            );
                        }
                        for (index, entry) in self.transcript.entries.iter().enumerate() {
                            match entry {
                                Entry::User { text } => {
                                    let (where_, words) = split_context(text);
                                    let rect = ui
                                        .with_layout(
                                            egui::Layout::top_down(egui::Align::Max),
                                            |ui| {
                                                ui.spacing_mut().item_spacing.y = 3.0;
                                                if let Some(where_) = where_ {
                                                    // `playhead 3:12.000 · section …`
                                                    // reads as `at 3:12.000`; the
                                                    // whole line is the tooltip.
                                                    let at = where_
                                                        .split(" \u{00b7} ")
                                                        .next()
                                                        .unwrap_or(where_)
                                                        .replacen("playhead ", "at ", 1);
                                                    ui.add(
                                                        egui::Label::new(micro(&at, c.weak))
                                                            .truncate(),
                                                    )
                                                    .on_hover_text(where_);
                                                }
                                                let width = ui.available_width() * 0.84;
                                                egui::Frame::new()
                                                    .fill(c.selected)
                                                    .corner_radius(egui::CornerRadius {
                                                        nw: 12,
                                                        ne: 12,
                                                        sw: 12,
                                                        se: 3,
                                                    })
                                                    .inner_margin(egui::Margin::symmetric(12, 9))
                                                    .show(ui, |ui| {
                                                        ui.set_max_width(width);
                                                        ui.with_layout(
                                                            egui::Layout::top_down(
                                                                egui::Align::Min,
                                                            ),
                                                            |ui| {
                                                                ui.add(
                                                                    egui::Label::new(
                                                                        RichText::new(words)
                                                                            .color(c.ink),
                                                                    )
                                                                    .wrap(),
                                                                );
                                                            },
                                                        );
                                                    })
                                                    .response
                                                    .rect
                                            },
                                        )
                                        .inner;
                                    layout.push(("user", rect));
                                }
                                Entry::Assistant { text } => {
                                    let rect =
                                        ui.scope(|ui| assistant_text(ui, text)).response.rect;
                                    layout.push(("assistant", rect));
                                }
                                Entry::Tool {
                                    name,
                                    detail,
                                    state,
                                    output: result,
                                    ..
                                } => {
                                    let colour = match state {
                                        ToolState::Failed => c.danger,
                                        _ => c.weak,
                                    };
                                    let response = egui::Frame::new()
                                        .fill(c.trough)
                                        .stroke(Stroke::new(1.0, c.rule))
                                        .corner_radius(6.0)
                                        .inner_margin(egui::Margin::symmetric(9, 6))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 8.0;
                                                ui.label(
                                                    RichText::new(tool_glyph(name))
                                                        .size(11.0)
                                                        .color(colour),
                                                );
                                                ui.label(
                                                    RichText::new(name).size(12.0).color(colour),
                                                );
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(detail)
                                                            .font(mono.clone())
                                                            .color(c.ink),
                                                    )
                                                    .truncate(),
                                                );
                                                if *state == ToolState::Running {
                                                    ui.label(
                                                        RichText::new("\u{2026}").color(c.weak),
                                                    );
                                                }
                                            });
                                        })
                                        .response
                                        .interact(egui::Sense::click());
                                    layout.push(("tool", response.rect));
                                    if !result.trim().is_empty() {
                                        let response =
                                            response.on_hover_text("Show what it returned");
                                        if response.clicked() {
                                            toggled = Some(index);
                                        }
                                        if self.expanded.contains(&index) {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(result.as_str())
                                                        .font(mono.clone())
                                                        .color(c.weak),
                                                )
                                                .wrap(),
                                            );
                                        }
                                    }
                                }
                                Entry::Permission {
                                    tool,
                                    detail,
                                    allowed,
                                    ..
                                } => {
                                    // The warning outline marks only a prompt still
                                    // waiting on you; answered, it is a record.
                                    let edge = if allowed.is_none() { c.warning } else { c.rule };
                                    let head = if allowed.is_none() { c.warning } else { c.weak };
                                    let card = egui::Frame::new()
                                        .fill(c.raised)
                                        .stroke(Stroke::new(1.0, edge))
                                        .corner_radius(6.0)
                                        .inner_margin(egui::Margin::same(12))
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.spacing_mut().item_spacing.y = 8.0;
                                            ui.horizontal(|ui| {
                                                let (dot, _) = ui.allocate_exact_size(
                                                    egui::vec2(6.0, 6.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().circle_filled(dot.center(), 3.0, head);
                                                ui.label(micro(
                                                    &format!(
                                                        "{} asks to {}",
                                                        self.agent.label(),
                                                        if tool == "Bash" { "run" } else { "use" }
                                                    ),
                                                    head,
                                                ));
                                                if tool != "Bash" {
                                                    ui.label(micro(tool, c.ink));
                                                }
                                            });
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(detail.as_str())
                                                        .font(mono.clone())
                                                        .color(c.ink),
                                                )
                                                .wrap(),
                                            );
                                            match allowed {
                                                None => {
                                                    ui.horizontal(|ui| {
                                                        let allow = egui::Button::new(
                                                            RichText::new("Allow").color(c.surface),
                                                        )
                                                        .fill(c.ink)
                                                        .corner_radius(4.0)
                                                        .min_size(egui::vec2(60.0, 28.0));
                                                        let allow = ui.add(allow);
                                                        layout.push(("allow", allow.rect));
                                                        if allow.clicked() {
                                                            decisions.push((index, true));
                                                        }
                                                        let deny = egui::Button::new(
                                                            RichText::new("Deny").color(c.weak),
                                                        )
                                                        .frame(false)
                                                        .min_size(egui::vec2(52.0, 28.0));
                                                        if ui.add(deny).clicked() {
                                                            decisions.push((index, false));
                                                        }
                                                    });
                                                }
                                                Some(true) => {
                                                    ui.label(micro("allowed", c.good));
                                                }
                                                Some(false) => {
                                                    ui.label(micro("denied", c.danger));
                                                }
                                            }
                                        });
                                    layout.push(("permission", card.response.rect));
                                }
                                Entry::Notice { text, error } => {
                                    let text = self.note_times.get(&index).map_or_else(
                                        || text.clone(),
                                        |at| {
                                            format!("{text} · {}", note_age(at.elapsed().as_secs()))
                                        },
                                    );
                                    let line = ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        let tone = if *error { c.danger } else { c.good };
                                        let (dot, _) = ui.allocate_exact_size(
                                            egui::vec2(6.0, 6.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().circle_filled(dot.center(), 3.0, tone);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(text.as_str())
                                                    .size(12.0)
                                                    .color(if *error { c.danger } else { c.weak }),
                                            )
                                            .wrap(),
                                        );
                                    });
                                    layout.push(("notice", line.response.rect));
                                }
                            }
                        }
                        if self.transcript.busy && !self.transcript.awaiting_permission() {
                            ui.label(RichText::new("\u{2026}").color(c.weak));
                        }
                    });
            });
        if let Some(index) = toggled {
            if !self.expanded.remove(&index) {
                self.expanded.insert(index);
            }
        }
        for (index, allow) in decisions {
            let lines = self.transcript.decide(index, allow);
            if let Some(process) = self.process.as_mut() {
                for line in lines {
                    let _ = process.write_line(&line);
                }
            }
        }

        // The composer: a rounded box, the words, then the playhead chip,
        // `Enter sends` and a round ink send button.
        let mut composer_ui = ui.new_child(egui::UiBuilder::new().max_rect(composer_rect));
        let mut send_rect = egui::Rect::NOTHING;
        let composer_id = self.id.with("composer");
        let mut send = false;
        if self.composer_focused
            && composer_ui.input_mut(|input| {
                // egui's consume_key(NONE) also matches Shift+Enter. Leave
                // that event for TextEdit so it inserts a newline.
                let mut send = false;
                input.events.retain(|event| {
                    let matched = matches!(event, egui::Event::Key {
                        key: Key::Enter, pressed: true, modifiers, ..
                    } if *modifiers == Modifiers::NONE);
                    send |= matched;
                    !matched
                });
                send
            })
        {
            send = true;
        }
        let mut picks = Vec::new();
        let composer = egui::Frame::new()
            .fill(c.raised)
            .stroke(Stroke::new(1.0, of_edge()))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(&mut composer_ui, |ui| {
                ui.set_width(ui.available_width());
                // Where you are comes first, as what the message starts with.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let playhead = context
                        .now
                        .split(" \u{00b7} ")
                        .next()
                        .filter(|part| !part.is_empty())
                        .unwrap_or("playhead");
                    let chip = egui::Button::selectable(
                        self.with_context,
                        RichText::new(playhead).size(12.0),
                    )
                    .corner_radius(12.0);
                    if ui
                        .add(chip)
                        .on_hover_text(format!(
                            "Start the message with where you are: [Musializer: {}]",
                            if context.now.is_empty() {
                                "no track open"
                            } else {
                                &context.now
                            }
                        ))
                        .clicked()
                    {
                        self.with_context = !self.with_context;
                    }
                    // Only where it fits beside Stop.
                    if ui.available_width() > 150.0 {
                        ui.label(micro("Enter sends", c.weak));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.transcript.busy
                            && ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Stop").size(12.0).color(c.weak),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Interrupt the turn")
                                .clicked()
                        {
                            let lines = self.transcript.interrupt();
                            if lines.is_empty() {
                                self.close();
                                self.transcript.busy = false;
                            } else {
                                self.write(lines);
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                // Keep the newest input visible without displacing Send or
                // crossing into the terminal pane below this fixed composer.
                let response = egui::ScrollArea::vertical()
                    .id_salt(self.id.with("composer_scroll"))
                    .max_height(22.0)
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.composer)
                                .id(composer_id)
                                .frame(egui::Frame::NONE)
                                .margin(egui::Margin::ZERO)
                                .desired_rows(1)
                                .desired_width(f32::INFINITY)
                                .hint_text(
                                    RichText::new(format!(
                                        "Ask {} to direct\u{2026}",
                                        self.agent.label()
                                    ))
                                    .color(c.weak),
                                ),
                        )
                    })
                    .inner;
                if std::mem::take(&mut self.focus_next) {
                    response.request_focus();
                }
                self.composer_focused = response.has_focus();
                output.has_focus = response.has_focus();
                layout.push(("entry", response.rect));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CHIP_GAP;
                    let menus = self.control_chips(ui, &c, &mut layout, &mut picks);
                    // A menu takes Escape, which the shell would otherwise use.
                    output.has_focus |= menus;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = !self.composer.trim().is_empty();
                        let (rect, button) =
                            ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                        let fill = if enabled { c.ink } else { c.rule };
                        ui.painter().circle_filled(rect.center(), 14.0, fill);
                        // An up arrow, drawn: the interface face has none.
                        let centre = rect.center();
                        let stroke = Stroke::new(1.8, c.surface);
                        ui.painter().line_segment(
                            [
                                centre + egui::vec2(0.0, 6.0),
                                centre + egui::vec2(0.0, -6.0),
                            ],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [
                                centre + egui::vec2(-5.0, -1.0),
                                centre + egui::vec2(0.0, -6.0),
                            ],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [
                                centre + egui::vec2(5.0, -1.0),
                                centre + egui::vec2(0.0, -6.0),
                            ],
                            stroke,
                        );
                        send_rect = rect;
                        if enabled && button.on_hover_text("Send [Enter]").clicked() {
                            send = true;
                        }
                    });
                });
            });
        for pick in picks {
            self.pick(pick);
        }
        layout.push(("composer", composer.response.rect));
        layout.push(("send", send_rect));
        self.layout = layout;
        if send && !self.composer.trim().is_empty() {
            self.send(context);
            composer_ui.memory_mut(|memory| memory.request_focus(composer_id));
        }
        ui.advance_cursor_after_rect(full);
        output
    }

    /// The composer's controls: model, effort, access. Returns whether one of
    /// their menus is open.
    fn control_chips(
        &mut self,
        ui: &mut egui::Ui,
        c: &ChatInk,
        layout: &mut Vec<(&'static str, egui::Rect)>,
        picks: &mut Vec<Pick>,
    ) -> bool {
        self.menu_rows.clear();
        let model_text = self.transcript.model_label();
        let effort_text = self.transcript.effort_label();
        let access = self.transcript.selection.access;
        let level = self.transcript.current_effort();
        let efforts = self
            .transcript
            .current_model()
            .map(|model| model.efforts.clone())
            .unwrap_or_default();

        let plain = |text: &str| natural_width(ui, text);
        let model_width = CHIP_PADDING * 2.0 + plain(&model_text);
        let effort_width = CHIP_PADDING * 2.0 + GLYPH_WIDTH + GLYPH_GAP + plain(&effort_text);
        let bare_access = CHIP_PADDING * 2.0 + GLYPH_WIDTH;
        let access_width = bare_access + GLYPH_GAP + plain(access.chip());
        // The send button and the three gaps stay; the model's name is cut
        // first, then the access chip keeps only its glyph.
        let room = ui.available_width() - 28.0 - CHIP_GAP * 3.0;
        let worded = room - effort_width - access_width >= MODEL_CHIP_LEAST;
        let access_width = if worded { access_width } else { bare_access };
        let model_width = model_width.min((room - effort_width - access_width).max(40.0));

        let menu_id = self.id.with("composer_menu");
        let open =
            |ui: &egui::Ui, name: &str| egui::Popup::is_id_open(ui.ctx(), menu_id.with(name));

        let model_open = open(ui, "model");
        let model = chip(
            ui,
            model_width,
            ChipGlyph::None,
            Some(&model_text),
            true,
            model_open,
            c,
        )
        .on_hover_text("Choose the model");
        layout.push(("chip-model", model.rect));
        let effort_open = open(ui, "effort");
        let effort = chip(
            ui,
            effort_width,
            ChipGlyph::Level(level_of(level.as_deref())),
            Some(&effort_text),
            !efforts.is_empty(),
            effort_open,
            c,
        );
        let effort = if efforts.is_empty() {
            effort.on_hover_text(format!("{model_text} has no effort levels"))
        } else {
            effort.on_hover_text("Choose how hard it thinks")
        };
        layout.push(("chip-effort", effort.rect));
        let access_open = open(ui, "access");
        let access_chip = chip(
            ui,
            access_width,
            ChipGlyph::Access(access),
            worded.then_some(access.chip()),
            true,
            access_open,
            c,
        )
        .on_hover_text(format!("{}: {}", access.label(), access.description()));
        layout.push(("chip-access", access_chip.rect));

        let mut rows = Vec::new();
        let note = self.landing_note();
        let note = note.as_ref();
        menu(&model, menu_id.with("model"), |ui| {
            ui.label(micro(&format!("{} model", self.agent.label()), c.weak));
            if self.transcript.models.is_empty() {
                let text = self.transcript.models_error.as_deref().map_or_else(
                    || "Listing the models\u{2026}".to_owned(),
                    |error| format!("Could not list the models: {error}"),
                );
                ui.add(egui::Label::new(RichText::new(text).color(c.weak)).wrap());
                ui.ctx().request_repaint();
            }
            let current = self.transcript.current_model().map(|m| m.id.clone());
            egui::ScrollArea::vertical()
                .max_height(MODEL_MENU_HEIGHT)
                .show(ui, |ui| {
                    for model in &self.transcript.models {
                        let row = menu_row(
                            ui,
                            ChipGlyph::None,
                            &model.label,
                            Some(model.description.as_str()).filter(|text| !text.is_empty()),
                            current.as_ref() == Some(&model.id),
                            true,
                            c,
                        );
                        rows.push((format!("model:{}", model.id), row.rect));
                        if row.clicked() {
                            picks.push(Pick::Model(model.id.clone()));
                            ui.close();
                        }
                    }
                });
            landing(ui, note, c);
        });
        menu(&effort, menu_id.with("effort"), |ui| {
            ui.label(micro("Effort", c.weak));
            for each in &efforts {
                let row = menu_row(
                    ui,
                    ChipGlyph::Level(level_of(Some(each))),
                    &effort_title(each),
                    None,
                    level.as_ref() == Some(each),
                    true,
                    c,
                );
                rows.push((format!("effort:{each}"), row.rect));
                if row.clicked() {
                    picks.push(Pick::Effort(each.clone()));
                    ui.close();
                }
            }
            landing(ui, note, c);
        });
        menu(&access_chip, menu_id.with("access"), |ui| {
            ui.label(micro("Access", c.weak));
            for each in Access::ALL {
                let refusal = self.transcript.access_refusal(each);
                let row = menu_row(
                    ui,
                    ChipGlyph::Access(each),
                    each.label(),
                    Some(refusal.as_deref().unwrap_or(each.description())),
                    each == access,
                    refusal.is_none(),
                    c,
                );
                rows.push((format!("access:{}", each.key()), row.rect));
                if row.clicked() {
                    picks.push(Pick::Access(each));
                    ui.close();
                }
            }
            landing(ui, note, c);
        });
        self.menu_rows = rows;
        model_open || effort_open || access_open
    }

    /// When a choice reaches this agent, where that is not at once.
    fn landing_note(&self) -> Option<String> {
        match self.transcript.protocol {
            Protocol::Claude => None,
            Protocol::Codex => Some("Takes effect with your next message.".to_owned()),
            Protocol::Antigravity => Some(
                "Takes effect with your next message, which restarts Antigravity and keeps \
                 the conversation."
                    .to_owned(),
            ),
        }
    }

    /// What an empty chat says first: who it is, where it works, and what
    /// it already knows, so the first message can be about the music.
    fn first_words(&self, ui: &mut egui::Ui, context: &DockContext, c: &ChatInk) {
        ui.label(
            RichText::new(format!(
                "{} directs from {}.",
                self.agent.label(),
                context.place
            ))
            .color(c.ink),
        );
        ui.label(
            RichText::new(match &context.following {
                Some(following) => format!(
                    "The window follows {}: each save the agent makes reaches the preview at the playhead.",
                    following.name
                ),
                None => "Ask for a look, a section, a still or a clip. It reads the project, \
                         writes a treatment, and the preview follows it."
                    .to_owned(),
            })
            .color(c.weak),
        );
    }
}

/// What a chip draws before its words.
#[derive(Clone, Copy)]
enum ChipGlyph {
    None,
    /// Filled bars out of five.
    Level(usize),
    Access(Access),
}

/// Where an effort level sits on five bars. A level the CLI invents later
/// draws empty rather than as a level it is not.
fn level_of(level: Option<&str>) -> usize {
    match level {
        Some("low") => 1,
        Some("medium") => 2,
        Some("high") => 3,
        Some("xhigh") => 4,
        Some("max" | "ultra") => 5,
        _ => 0,
    }
}

fn natural_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        )
        .size()
        .x
        .ceil()
}

/// Glyphs are painted, as the send arrow is: the interface face has none.
fn paint_glyph(
    painter: &egui::Painter,
    glyph: ChipGlyph,
    at: egui::Pos2,
    tone: egui::Color32,
    off: egui::Color32,
) {
    let stroke = Stroke::new(1.3, tone);
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points
                .iter()
                .map(|(x, y)| at + egui::vec2(*x, *y))
                .collect(),
            stroke,
        ));
    };
    match glyph {
        ChipGlyph::None => {}
        ChipGlyph::Level(level) => {
            for bar in 0..5_usize {
                let height = 3.0 + 2.0 * bar as f32;
                let left = at.x + bar as f32 * 3.0 - 7.0;
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left, at.y + 6.0 - height),
                    egui::pos2(left + 2.0, at.y + 6.0),
                );
                painter.rect_filled(rect, 0.0, if bar < level { tone } else { off });
            }
        }
        ChipGlyph::Access(Access::Supervised | Access::Full) => {
            let full = matches!(glyph, ChipGlyph::Access(Access::Full));
            painter.rect_stroke(
                egui::Rect::from_min_max(at + egui::vec2(-4.5, -1.0), at + egui::vec2(4.5, 6.0)),
                1.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            // Open, the shackle stands clear of the body on one side.
            let left = if full { 0.5 } else { -2.5 };
            let foot = if full { -3.0 } else { -1.0 };
            line(&[
                (left, -1.0),
                (left, -4.0),
                (left + 1.0, -5.5),
                (left + 4.0, -5.5),
                (left + 5.0, -4.0),
                (left + 5.0, foot),
            ]);
        }
        ChipGlyph::Access(Access::AcceptEdits) => {
            line(&[
                (-4.5, 5.5),
                (-3.8, 2.5),
                (2.5, -4.0),
                (4.8, -1.8),
                (-1.5, 4.8),
                (-4.5, 5.5),
            ]);
        }
        ChipGlyph::Access(Access::Auto) => {
            line(&[
                (0.0, -6.0),
                (1.4, -1.4),
                (6.0, 0.0),
                (1.4, 1.4),
                (0.0, 6.0),
                (-1.4, 1.4),
                (-6.0, 0.0),
                (-1.4, -1.4),
                (0.0, -6.0),
            ]);
        }
    }
}

/// A ghost control: its words and glyph until hovered or open.
fn chip(
    ui: &mut egui::Ui,
    width: f32,
    glyph: ChipGlyph,
    text: Option<&str>,
    enabled: bool,
    open: bool,
    c: &ChatInk,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, CHIP_HEIGHT), sense);
    let hot = enabled && (response.hovered() || open);
    if hot {
        ui.painter().rect_filled(rect, 6.0, c.selected);
    }
    let tone = if hot { c.ink } else { c.weak };
    let mut left = rect.left() + CHIP_PADDING;
    if !matches!(glyph, ChipGlyph::None) {
        let at = egui::pos2(left + GLYPH_WIDTH / 2.0, rect.center().y);
        paint_glyph(ui.painter(), glyph, at, tone, c.rule);
        left += GLYPH_WIDTH + GLYPH_GAP;
    }
    if let Some(text) = text {
        let mut job =
            LayoutJob::simple_singleline(text.to_owned(), egui::FontId::proportional(12.0), tone);
        job.wrap = TextWrapping::truncate_at_width((rect.right() - CHIP_PADDING - left).max(0.0));
        let galley = ui.painter().layout_job(job);
        let top = rect.center().y - galley.size().y / 2.0;
        ui.painter().galley(egui::pos2(left, top), galley, tone);
    }
    response
}

/// One of a chip's menus: above the composer, since the composer is at the
/// bottom of the window. A click in it keeps it open until a row is taken, so
/// a refused row can be read.
fn menu(button: &egui::Response, id: egui::Id, content: impl FnOnce(&mut egui::Ui)) {
    egui::Popup::menu(button)
        .id(id)
        .align(egui::RectAlign::TOP_START)
        .gap(6.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(MENU_WIDTH);
            ui.spacing_mut().item_spacing.y = 2.0;
            content(ui);
        });
}

/// A menu row: its glyph, its name over one line of description, and a tick
/// on the one in use.
fn menu_row(
    ui: &mut egui::Ui,
    glyph: ChipGlyph,
    name: &str,
    description: Option<&str>,
    active: bool,
    enabled: bool,
    c: &ChatInk,
) -> egui::Response {
    let height = if description.is_some() { 44.0 } else { 30.0 };
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), sense);
    let hot = enabled && response.hovered();
    if hot {
        ui.painter().rect_filled(rect, 4.0, c.selected);
    }
    let painter = ui.painter();
    let tone = if enabled { c.ink } else { c.weak };
    let mut left = rect.left() + 8.0;
    if !matches!(glyph, ChipGlyph::None) {
        let at = egui::pos2(left + GLYPH_WIDTH / 2.0, rect.center().y);
        paint_glyph(painter, glyph, at, tone, c.rule);
        left += GLYPH_WIDTH + GLYPH_GAP + 3.0;
    }
    let tick_room = 22.0;
    let text = |words: &str, size: f32, colour: egui::Color32, top: f32| {
        let mut job = LayoutJob::simple_singleline(
            words.to_owned(),
            egui::FontId::proportional(size),
            colour,
        );
        job.wrap = TextWrapping::truncate_at_width((rect.right() - tick_room - left).max(0.0));
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(left, top), galley, colour);
    };
    match description {
        Some(description) => {
            text(name, 13.0, tone, rect.top() + 6.0);
            text(description, 11.0, c.weak, rect.top() + 25.0);
        }
        None => text(name, 13.0, tone, rect.top() + 7.0),
    }
    if active {
        let at = egui::pos2(rect.right() - 14.0, rect.center().y);
        painter.add(egui::Shape::line(
            vec![
                at + egui::vec2(-4.0, 0.0),
                at + egui::vec2(-1.0, 3.0),
                at + egui::vec2(4.0, -3.5),
            ],
            Stroke::new(1.6, c.ink),
        ));
    }
    response.on_hover_text(description.unwrap_or(name))
}

/// The line under a menu that says when its choice lands.
fn landing(ui: &mut egui::Ui, note: Option<&String>, c: &ChatInk) {
    if let Some(note) = note {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(note.as_str()).size(11.0).color(c.weak)).wrap());
    }
}

/// The controls' edge in egui.
fn of_edge() -> egui::Color32 {
    crate::ui::theme::color32::of(crate::ui::theme::palette().ui_control_edge)
}

#[cfg(test)]
mod polish_tests {
    #[test]
    fn failed_restart_preserves_draft_and_does_not_start_a_turn() {
        let context = super::DockContext {
            // A nonexistent cwd makes launch fail even on machines with a CLI.
            working_directory: Some("/nonexistent/musializer-chat-test".into()),
            ..super::DockContext::default()
        };
        let mut chat = super::ChatTab::new(
            super::Agent::Claude,
            &context,
            egui::Id::new("failed"),
            super::Selection::default(),
        );
        chat.composer = "keep this prompt".into();
        let before = chat.transcript.counts();
        chat.send(&context);
        assert!(chat.process.is_none());
        assert!(chat.error.is_some());
        assert_eq!(chat.composer, "keep this prompt");
        assert_eq!(chat.transcript.counts(), before);
        assert!(!chat.transcript.busy);
    }

    #[test]
    fn treatment_note_age_uses_elapsed_wall_time() {
        assert_eq!(super::note_age(0), "0 s ago");
        assert_eq!(super::note_age(40), "40 s ago");
        assert_eq!(super::note_age(59), "59 s ago");
        assert_eq!(super::note_age(60), "1 min ago");
        assert_eq!(super::note_age(3600), "1 h ago");
    }
}
