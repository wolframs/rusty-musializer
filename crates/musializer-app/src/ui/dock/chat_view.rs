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
use crate::ui::theme::{self, color32};

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
        let waiting = color32::of(theme::palette().ui_warning);
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
