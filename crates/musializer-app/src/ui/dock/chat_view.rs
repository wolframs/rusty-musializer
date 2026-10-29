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

use egui::{Key, Modifiers, RichText, Stroke};
use musializer_core::agent::{Entry, Protocol, ToolState, Transcript};
use musializer_runtime::agent_process::{AgentProcess, Launch, Output};

use super::{Agent, DockContext};
use crate::ui::theme::color32;

/// Whether this protocol's tool permissions can be asked for in a chat.
const fn asks_permission(protocol: Protocol) -> bool {
    !matches!(protocol, Protocol::Antigravity)
}

pub struct ChatTab {
    pub agent: Agent,
    transcript: Transcript,
    process: Option<AgentProcess>,
    composer: String,
    /// Prefix each message with the playhead line.
    with_context: bool,
    /// Antigravity only: start it with `--dangerously-skip-permissions`.
    allow_commands: bool,
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
    /// What the column's chat drew in its last frame, kind and rect, for the
    /// `dock:` line (WX-5d): every probe and pixel check reads its rects here.
    pub layout: Vec<(&'static str, egui::Rect)>,
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
    pub fn new(agent: Agent, context: &DockContext, id: egui::Id) -> Self {
        let mut tab = Self {
            agent,
            transcript: Transcript::new(protocol(agent)),
            process: None,
            composer: String::new(),
            with_context: true,
            allow_commands: false,
            preamble_pending: agent == Agent::Antigravity,
            instructions: context.instructions.clone(),
            error: None,
            focus_next: true,
            composer_focused: false,
            id,
            expanded: std::collections::HashSet::new(),
            layout: Vec::new(),
        };
        tab.start(context, None);
        tab
    }

    fn launch(&self, context: &DockContext, resume: Option<&str>) -> Launch {
        let override_var = format!("MUSIALIZER_AGENT_{}", self.agent.command().to_uppercase());
        let overridden = std::env::var(&override_var).ok().filter(|v| !v.is_empty());
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
                    "--permission-mode",
                    "acceptEdits",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
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
                    "--mode",
                    "accept-edits",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect();
                if self.allow_commands {
                    args.push("--dangerously-skip-permissions".into());
                }
                if let Some(session) = resume {
                    args.push("--conversation".into());
                    args.push(session.to_string());
                }
                // Last, attached: agy's `-p` takes the next word as the prompt.
                args.push("--print=".into());
                args
            }
        };
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
                Output::Diagnostic(_) => {}
                Output::Exited(how) => self.transcript.exited(&how),
            }
        }
    }

    pub fn close(&mut self) {
        if let Some(process) = self.process.as_mut() {
            process.close();
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
        if self.process.as_ref().is_none_or(AgentProcess::has_exited) {
            let resume = self.transcript.session.clone();
            self.start(context, resume.as_deref());
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
        format!(
            "chat users={users} replies={replies} tools={tools} prompts={prompts} busy={} model={} last={:?}{}",
            self.transcript.busy,
            self.transcript.model.as_deref().unwrap_or("?"),
            self.transcript.last_reply().unwrap_or("").chars().take(120).collect::<String>(),
            self.error
                .as_deref()
                .map_or_else(String::new, |error| format!(" error={error:?}")),
        )
    }

    pub fn show(&mut self, ui: &mut egui::Ui, context: &DockContext) -> ChatOutput {
        let mut output = ChatOutput::default();
        // The palette's roles (WX-5a), not literals: these sit on the dock's
        // surface, which is a different colour in each appearance.
        let weak = ui.visuals().weak_text_color();
        let ink = color32::ui_ink();
        let waiting = color32::ui_warning();
        let danger = color32::ui_danger();
        let good = color32::ui_success();

        // Status line.
        ui.horizontal(|ui| {
            let status = if let Some(error) = &self.error {
                RichText::new(format!("could not start: {error}")).color(danger)
            } else if self.process.as_ref().is_some_and(AgentProcess::has_exited) {
                RichText::new("stopped · sending starts it again").color(weak)
            } else if self.transcript.awaiting_permission() {
                RichText::new("waiting for you").color(waiting)
            } else if self.transcript.busy {
                RichText::new("working…").color(ink)
            } else {
                RichText::new("ready").color(good)
            };
            ui.label(status.small());
            if let Some(model) = &self.transcript.model {
                ui.label(RichText::new(model).small().color(weak));
            }
            if let Some(usage) = &self.transcript.usage {
                ui.label(RichText::new(usage).small().color(weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.transcript.busy
                    && ui
                        .small_button("Stop")
                        .on_hover_text("Interrupt the turn")
                        .clicked()
                {
                    let lines = self.transcript.interrupt();
                    if lines.is_empty() {
                        // No interrupt in this protocol: end the process; the
                        // next message resumes the conversation.
                        self.close();
                        self.transcript.busy = false;
                    } else if let Some(process) = self.process.as_mut() {
                        for line in lines {
                            let _ = process.write_line(&line);
                        }
                    }
                }
            });
        });
        ui.separator();

        // The composer goes at the bottom; reserve its two text rows, the row
        // of toggles under it and the separators, in this theme's sizes.
        let row = ui.text_style_height(&egui::TextStyle::Body);
        let spacing = ui.spacing();
        let composer_height =
            row * 2.0 + spacing.interact_size.y + spacing.item_spacing.y * 4.0 + 24.0;
        let list_height = (ui.available_height() - composer_height).max(60.0);
        let mut decisions = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt(self.id.with("scroll"))
            .max_height(list_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.transcript.entries.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} runs in {} and knows the window follows {}.",
                            self.agent.label(),
                            context.place,
                            context
                                .following
                                .as_ref()
                                .map_or("no treatment yet", |f| f.name.as_str())
                        ))
                        .color(weak),
                    );
                }
                for (index, entry) in self.transcript.entries.iter().enumerate() {
                    match entry {
                        Entry::User { text } => {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                egui::Frame::new()
                                    .fill(ui.visuals().faint_bg_color)
                                    .stroke(Stroke::new(
                                        1.0,
                                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                                    ))
                                    .corner_radius(8.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.set_max_width(ui.available_width() * 0.85);
                                        ui.add(egui::Label::new(text.as_str()).wrap());
                                    });
                            });
                        }
                        Entry::Assistant { text } => assistant_text(ui, text),
                        Entry::Tool {
                            name,
                            detail,
                            state,
                            output,
                            ..
                        } => {
                            // Glyphs every bundled egui face has: a check
                            // mark is not among them and drew as a box.
                            let (glyph, colour) = match state {
                                ToolState::Running => ("…", ink),
                                ToolState::Done => ("•", good),
                                ToolState::Failed => ("×", danger),
                            };
                            let head = RichText::new(format!("{glyph} {name}  {detail}"))
                                .small()
                                .monospace()
                                .color(if *state == ToolState::Failed {
                                    danger
                                } else {
                                    colour
                                });
                            if output.trim().is_empty() {
                                ui.add(egui::Label::new(head).truncate());
                            } else {
                                egui::CollapsingHeader::new(head)
                                    .id_salt(self.id.with(("tool", index)))
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(output.as_str())
                                                    .small()
                                                    .monospace()
                                                    .color(weak),
                                            )
                                            .wrap(),
                                        );
                                    });
                            }
                        }
                        Entry::Permission {
                            tool,
                            detail,
                            allowed,
                            ..
                        } => {
                            egui::Frame::new()
                                .stroke(Stroke::new(1.0, waiting))
                                .corner_radius(6.0)
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.label(format!(
                                        "{} wants to use {tool}:",
                                        self.agent.label()
                                    ));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(detail.as_str()).monospace().small(),
                                        )
                                        .wrap(),
                                    );
                                    match allowed {
                                        None => {
                                            ui.horizontal(|ui| {
                                                if ui.button("Allow").clicked() {
                                                    decisions.push((index, true));
                                                }
                                                if ui.button("Deny").clicked() {
                                                    decisions.push((index, false));
                                                }
                                            });
                                        }
                                        Some(true) => {
                                            ui.label(RichText::new("allowed").small().color(good));
                                        }
                                        Some(false) => {
                                            ui.label(RichText::new("denied").small().color(danger));
                                        }
                                    }
                                });
                        }
                        Entry::Notice { text, error } => {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(text.as_str()).small().color(if *error {
                                        danger
                                    } else {
                                        weak
                                    }),
                                )
                                .wrap(),
                            );
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        for (index, allow) in decisions {
            let lines = self.transcript.decide(index, allow);
            if let Some(process) = self.process.as_mut() {
                for line in lines {
                    let _ = process.write_line(&line);
                }
            }
        }

        ui.separator();
        // Enter sends and Shift+Enter is a new line, as in every chat. The key
        // is taken before the text field sees it.
        let composer_id = self.id.with("composer");
        let mut send = false;
        if self.composer_focused
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
        {
            send = true;
        }
        let hint = format!(
            "Ask {} to direct…  (Enter sends, Shift+Enter is a new line)",
            self.agent.label()
        );
        let response = ui.add(
            egui::TextEdit::multiline(&mut self.composer)
                .id(composer_id)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text(hint),
        );
        if std::mem::take(&mut self.focus_next) {
            response.request_focus();
        }
        self.composer_focused = response.has_focus();
        output.has_focus = response.has_focus();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.with_context, "playhead")
                .on_hover_text(format!(
                    "Start the message with where you are: [Musializer: {}]",
                    if context.now.is_empty() {
                        "no track open"
                    } else {
                        &context.now
                    }
                ));
            if !asks_permission(self.transcript.protocol) {
                let before = self.allow_commands;
                ui.checkbox(&mut self.allow_commands, "let it run commands")
                    .on_hover_text(
                        "Antigravity cannot ask for permission from a chat. This restarts \
                         it with --dangerously-skip-permissions, keeping the conversation.",
                    );
                if before != self.allow_commands && self.process.is_some() {
                    let resume = self.transcript.session.clone();
                    self.start(context, resume.as_deref());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = !self.composer.trim().is_empty();
                if ui.add_enabled(enabled, egui::Button::new("Send")).clicked() {
                    send = true;
                }
            });
        });
        if send {
            self.send(context);
            response.request_focus();
        }
        output
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
    pub fn note(&mut self, text: String) {
        self.transcript
            .entries
            .push(Entry::Notice { text, error: false });
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
        let composer_height = 92.0;
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
            && composer_ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
        {
            send = true;
        }
        let composer = egui::Frame::new()
            .fill(c.raised)
            .stroke(Stroke::new(1.0, of_edge()))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(&mut composer_ui, |ui| {
                ui.set_width(ui.available_width());
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.composer)
                        .id(composer_id)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_rows(1)
                        .desired_width(f32::INFINITY)
                        .hint_text(
                            RichText::new(format!("Ask {} to direct\u{2026}", self.agent.label()))
                                .color(c.weak),
                        ),
                );
                if std::mem::take(&mut self.focus_next) {
                    response.request_focus();
                }
                self.composer_focused = response.has_focus();
                output.has_focus = response.has_focus();
                ui.add_space(6.0);
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
                    ui.label(micro("Enter sends", c.weak));
                    if !asks_permission(self.transcript.protocol) {
                        let before = self.allow_commands;
                        ui.checkbox(&mut self.allow_commands, "run commands")
                            .on_hover_text(
                                "Antigravity cannot ask for permission from a chat. This restarts \
                                 it with --dangerously-skip-permissions, keeping the conversation.",
                            );
                        if before != self.allow_commands && self.process.is_some() {
                            let resume = self.transcript.session.clone();
                            self.start(context, resume.as_deref());
                        }
                    }
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
                            } else if let Some(process) = self.process.as_mut() {
                                for line in lines {
                                    let _ = process.write_line(&line);
                                }
                            }
                        }
                    });
                });
            });
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

/// The controls' edge in egui.
fn of_edge() -> egui::Color32 {
    crate::ui::theme::color32::of(crate::ui::theme::palette().ui_control_edge)
}
