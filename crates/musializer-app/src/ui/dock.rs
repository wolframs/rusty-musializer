//! The dock: a right-hand region for terminals and the agents run in them
//! (WX-1, WX-2; design in `docs/WORKSTATION.md`).
//!
//! **Not modal**, unlike the egui Editor. The composition root hands the
//! workspace a window narrower by [`Dock::width_for`], so every raylib panel
//! lays itself out beside the dock and no hit test changes. The dock is drawn
//! in the same egui pass as the Editor and is built *before* it, so a key the
//! terminal used (Escape above all, which interrupts an agent) is gone from
//! the frame's input by the time the Editor looks for its own Escape.
//!
//! **Keyboard ownership** is the terminal's focus, carried one frame: the
//! egui backend is told to deliver keys while [`Dock::wants_keyboard`], and
//! the shell's [`TextEntrySurface::Dock`](super::shell) stands every global
//! shortcut down for the same frames. A click anywhere outside the terminal
//! hands the keyboard back.
//!
//! Tabs run the user's shell, or an agent CLI started *through* the shell
//! (`$SHELL -i -c 'claude; exec $SHELL -i'`), so the agent sees the same PATH
//! and rc files as in any other terminal, and quitting it leaves a shell in
//! the tab rather than a dead one.

#[path = "dock/chat_view.rs"]
pub mod chat_view;
#[path = "dock/column_view.rs"]
mod column_view;
#[path = "dock/keys.rs"]
pub mod keys;
#[path = "dock/terminal_view.rs"]
pub mod terminal_view;

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Context, Rect, RichText, Stroke};

use crate::ui::theme::color32;
use musializer_runtime::terminal::{GridSize, Session, Spawn};

use chat_view::ChatTab;
use terminal_view::TerminalView;

/// The dock's own bounds. 360 is about 44 columns, the least an agent's TUI
/// is still usable in.
pub const MINIMUM: f32 = 360.0;
pub const MAXIMUM: f32 = 1100.0;
/// The grab strip on the dock's left edge.
const GRIP: f32 = 6.0;

/// An agent CLI the dock can start in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
    Antigravity,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Antigravity];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
            Agent::Antigravity => "Antigravity",
        }
    }

    /// The agent named on a probe line: `claude`, `codex`, `agy` or
    /// `antigravity`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Agent::Claude),
            "codex" => Some(Agent::Codex),
            "agy" | "antigravity" => Some(Agent::Antigravity),
            _ => None,
        }
    }

    /// The command as typed at a prompt.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Antigravity => "agy",
        }
    }
}

/// Where new tabs start and what they are told about the window.
#[derive(Clone, Debug, Default)]
pub struct DockContext {
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    /// Shown in the header, so it is clear which folder a new tab opens in.
    pub place: String,
    /// The treatment the window follows (WX-3), if any.
    pub following: Option<Following>,
    /// Where the user is, for a chat message's context line (WX-4):
    /// `playhead 3:12.0 · section 38 of 70, threshold · treatment x.json`.
    pub now: String,
    /// The standing instructions a chat agent gets once per session.
    pub instructions: String,
}

/// What the header says about the followed treatment.
#[derive(Clone, Debug, Default)]
pub struct Following {
    pub name: String,
    pub applied: u32,
    /// Why the last change was refused, while it still stands.
    pub error: Option<String>,
}

/// A terminal tab: a shell, or an agent CLI run through one.
struct Terminal {
    agent: Option<Agent>,
    spawn: Spawn,
    session: Option<Session>,
    error: Option<String>,
    view: TerminalView,
}

#[allow(
    clippy::large_enum_variant,
    reason = "a dock holds a handful of tabs, created by a click; the size of one is irrelevant"
)]
enum Body {
    Terminal(Terminal),
    Chat(ChatTab),
}

struct Tab {
    label: String,
    body: Body,
    id: egui::Id,
}

impl Tab {
    fn title(&self) -> String {
        match &self.body {
            Body::Terminal(terminal) => terminal.title(&self.label),
            Body::Chat(chat) => chat.title(),
        }
    }

    fn exited(&self) -> bool {
        match &self.body {
            Body::Terminal(terminal) => terminal
                .session
                .as_ref()
                .is_some_and(|session| session.exit().is_some()),
            Body::Chat(_) => false,
        }
    }

    fn focus(&mut self) {
        match &mut self.body {
            Body::Terminal(terminal) => terminal.view.focus_next = true,
            Body::Chat(chat) => chat.focus_next = true,
        }
    }

    fn close(&mut self) {
        match &mut self.body {
            Body::Terminal(terminal) => {
                if let Some(session) = terminal.session.as_mut() {
                    session.close();
                }
            }
            Body::Chat(chat) => chat.close(),
        }
    }
}

impl Terminal {
    fn start(&mut self, focus: bool) {
        // The first resize sets the real size; this is only a starting grid.
        let size = GridSize {
            columns: 80,
            lines: 24,
            cell_width: 8,
            cell_height: 16,
        };
        match Session::spawn(&self.spawn, size) {
            Ok(session) => {
                self.session = Some(session);
                self.error = None;
            }
            Err(error) => {
                self.session = None;
                self.error = Some(error.to_string());
            }
        }
        self.view.focus_next = focus;
    }

    fn title(&self, label: &str) -> String {
        let live = self
            .session
            .as_ref()
            .and_then(Session::title)
            .filter(|title| !title.trim().is_empty());
        match (self.agent, live) {
            (Some(agent), _) => format!("{} (terminal)", agent.label()),
            (None, Some(title)) => title.chars().take(24).collect(),
            (None, None) => label.to_string(),
        }
    }
}

/// What a frame of the dock asks the composition root to do.
#[derive(Default)]
pub struct DockOutput {
    /// Hide the dock (Ctrl+` from inside a terminal).
    pub toggle: bool,
    /// The user finished dragging the dock to this width.
    pub resized: Option<f32>,
    /// Stop following the treatment (WX-3).
    pub stop_following: bool,
}

#[derive(Default)]
pub struct Dock {
    tabs: Vec<Tab>,
    active: usize,
    keyboard: bool,
    shells_opened: u32,
    dragging: Option<f32>,
    /// Ctrl+` opened the dock before it had a tab; the first tab takes focus.
    focus_first: bool,
    /// Whether the agent buttons open a native chat (the default) or the
    /// agent's own TUI in a terminal tab.
    agents_in_terminal: bool,
    opened: u64,
    /// Drawn as the column's Direct view (WX-5d): the column owns the grip,
    /// and the head is the column's 40 px head.
    in_column: bool,
    /// Agent replies the user has had on screen; more than this is an
    /// unseen reply, the Direct tab's ink dot.
    seen_replies: usize,
    /// A permission prompt has already posted its notice while away.
    waiting_noticed: bool,
    /// The Direct view's shown chat and terminal (WX-5d), as tab indices;
    /// a stale one falls back to the newest of its kind.
    chat_active: Option<usize>,
    terminal_active: Option<usize>,
    /// The Direct view's terminal pane: expanded, and its dragged height.
    terminal_open: bool,
    terminal_height: Option<f32>,
    /// The followed treatment's apply count already noted in the chat.
    applied_seen: u32,
    /// The empty state's Start buttons as last drawn, for `dock:`; empty
    /// while a chat is shown.
    start_rects: Vec<(&'static str, egui::Rect)>,
}

impl Dock {
    /// Whether a terminal held the keyboard at the end of the last frame.
    #[must_use]
    pub fn wants_keyboard(&self) -> bool {
        self.keyboard
    }

    /// Opens a tab. `None` is a plain shell. `focus` gives it the keyboard:
    /// yes for a tab the user just asked for, no for the one the dock opens
    /// by itself, because a dock restored open at launch must not take Space
    /// away from the transport before anyone has clicked into it.
    pub fn open_tab(&mut self, agent: Option<Agent>, context: &DockContext, focus: bool) {
        let program = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        let spawn = Spawn {
            program: agent.map(|agent| {
                (
                    program.clone(),
                    vec![
                        "-i".into(),
                        "-c".into(),
                        format!("{}; exec \"$SHELL\" -i", agent.command()),
                    ],
                )
            }),
            working_directory: context.working_directory.clone(),
            env: context.env.clone(),
        };
        let label = if agent.is_none() {
            self.shells_opened += 1;
            format!("Terminal {}", self.shells_opened)
        } else {
            String::new()
        };
        let mut terminal = Terminal {
            agent,
            spawn,
            session: None,
            error: None,
            view: TerminalView::default(),
        };
        terminal.start(focus);
        let id = self.next_id();
        self.tabs.push(Tab {
            label,
            body: Body::Terminal(terminal),
            id,
        });
        self.active = self.tabs.len() - 1;
        self.terminal_active = Some(self.active);
    }

    fn next_id(&mut self) -> egui::Id {
        self.opened += 1;
        egui::Id::new(("dock_tab", self.opened))
    }

    /// Opens a native chat with `agent` (WX-4).
    pub fn open_chat(&mut self, agent: Agent, context: &DockContext) {
        musializer_runtime::agent_process::warm_shell_path();
        let id = self.next_id();
        let chat = ChatTab::new(agent, context, id);
        self.tabs.push(Tab {
            label: agent.label().to_string(),
            body: Body::Chat(chat),
            id,
        });
        self.active = self.tabs.len() - 1;
        self.chat_active = Some(self.active);
    }

    /// Sends `text` to the active chat as if typed (`--ui-probe agent-say=`).
    pub fn probe_say(&mut self, text: &str, context: &DockContext) {
        let index = self.chat_index();
        if let Some(Body::Chat(chat)) = self.tabs.get_mut(index).map(|tab| &mut tab.body) {
            chat.probe_say(text, context);
        }
    }

    /// Answers the active chat's open permission prompts
    /// (`--ui-probe agent-decide=`).
    pub fn probe_decide(&mut self, allow: bool) {
        let index = self.chat_index();
        if let Some(Body::Chat(chat)) = self.tabs.get_mut(index).map(|tab| &mut tab.body) {
            chat.probe_decide(allow);
        }
    }

    /// Pumps every tab's process, visible or not, so an agent keeps working
    /// and a busy terminal's events do not pile up while the dock is hidden.
    pub fn tick(&mut self) {
        for tab in &mut self.tabs {
            match &mut tab.body {
                Body::Terminal(terminal) => {
                    if let Some(session) = terminal.session.as_mut() {
                        let _ = session.poll();
                    }
                }
                Body::Chat(chat) => chat.tick(),
            }
        }
    }

    /// Types `text` into the active terminal (`--ui-probe dock-run=`). Xvfb
    /// has no keyboard; key encoding is covered by `keys`' own tests.
    pub fn probe_write(&mut self, text: &str) {
        let index = self.terminal_index();
        if let Some(Body::Terminal(Terminal {
            session: Some(session),
            ..
        })) = self.tabs.get(index).map(|tab| &tab.body)
        {
            session.write(text.as_bytes().to_vec());
        }
    }

    /// The chat the probes and the Direct view address: the column's shown
    /// chat, or the dock's active tab.
    fn chat_index(&self) -> usize {
        if self.in_column {
            self.current_chat().unwrap_or(usize::MAX)
        } else {
            self.active
        }
    }

    /// The terminal the probes address, by the same rule.
    fn terminal_index(&self) -> usize {
        if self.in_column {
            self.current_terminal().unwrap_or(usize::MAX)
        } else {
            self.active
        }
    }

    /// Asks the active terminal to take the keyboard on the next frame.
    pub fn focus_active(&mut self) {
        // In the column the pane's terminal when it is open, else the chat's
        // composer: what Ctrl+` lands on is what the user would type into.
        let index = if self.in_column {
            if self.terminal_open {
                self.terminal_index()
            } else {
                self.chat_index()
            }
        } else {
            self.active
        };
        match self.tabs.get_mut(index) {
            Some(tab) => tab.focus(),
            None => self.focus_first = true,
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            let mut tab = self.tabs.remove(index);
            tab.close();
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            }
        }
    }

    /// Hangs up on every terminal. Called once, before the window closes.
    pub fn shutdown(&mut self) {
        while !self.tabs.is_empty() {
            self.close_tab(0);
        }
    }

    /// One line for the run report: the tabs, which is active, and the first
    /// non-empty line of its screen, so a capture's terminal content can be
    /// asserted without reading pixels.
    #[must_use]
    pub fn describe(&self, open: bool) -> String {
        if self.in_column {
            return self.describe_column(open);
        }
        if self.tabs.is_empty() {
            return format!("open={open} tabs=0");
        }
        let tabs: Vec<String> = self.tabs.iter().map(Tab::title).collect();
        let detail = match &self.tabs[self.active].body {
            Body::Terminal(terminal) => {
                let (state, screen) = terminal_state(terminal);
                format!("{state} keyboard={} screen={screen:?}", self.keyboard)
            }
            Body::Chat(chat) => format!("keyboard={} {}", self.keyboard, chat.describe()),
        };
        format!(
            "open={open} tabs={} [{}] active={} {detail}",
            tabs.len(),
            tabs.join(", "),
            self.active,
        )
    }

    /// Gives up the keyboard (WX-5d §2): the dock left the screen, so no
    /// surface of it may keep keystrokes. Also drops egui's focus, which
    /// would otherwise hand the keyboard straight back to the terminal the
    /// next time the dock is shown without anyone asking.
    pub fn release_keyboard(&mut self, ctx: &Context) {
        self.keyboard = false;
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }

    /// The Direct tab's dot, and a notice to post once when an agent starts
    /// waiting on Allow/Deny while the dock is away (WX-5d §2). `shown` is
    /// whether the dock is on screen this frame, which is when its replies
    /// count as seen.
    pub fn badge(&mut self, shown: bool) -> (crate::ui::column::DirectBadge, Option<String>) {
        use crate::ui::column::DirectBadge;
        let mut working = false;
        let mut waiting: Option<Agent> = None;
        let mut replies = 0;
        for tab in &self.tabs {
            if let Body::Chat(chat) = &tab.body {
                let (busy, asks, count) = chat.activity();
                working |= busy;
                if asks && waiting.is_none() {
                    waiting = Some(chat.agent);
                }
                replies += count;
            }
        }
        if shown {
            self.seen_replies = replies;
            self.waiting_noticed = waiting.is_some();
            return (DirectBadge::None, None);
        }
        // A closed tab can take replies with it; never count below zero.
        self.seen_replies = self.seen_replies.min(replies);
        let notice = match waiting {
            Some(agent) if !self.waiting_noticed => {
                self.waiting_noticed = true;
                Some(format!(
                    "{} asks to run a command \u{00b7} open Direct to answer",
                    agent.label()
                ))
            }
            Some(_) => None,
            None => {
                self.waiting_noticed = false;
                None
            }
        };
        let badge = if waiting.is_some() {
            DirectBadge::Waiting
        } else if replies > self.seen_replies {
            DirectBadge::Reply
        } else if working {
            DirectBadge::Working
        } else {
            DirectBadge::None
        };
        (badge, notice)
    }

    /// Builds the dock into `rect` (egui points).
    pub fn show(&mut self, ctx: &Context, rect: Rect, context: &DockContext) -> DockOutput {
        if self.in_column {
            return self.show_column(ctx, rect, context);
        }
        let mut output = DockOutput::default();
        if self.tabs.is_empty() {
            let focus = std::mem::take(&mut self.focus_first);
            self.open_tab(None, context, focus);
        }
        let visuals = ctx.global_style().visuals.clone();
        let surface = visuals.panel_fill;
        let rule = visuals.widgets.noninteractive.bg_stroke.color;
        egui::Area::new(egui::Id::new("musializer_dock"))
            .fixed_pos(rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_width(rect.width());
                ui.set_height(rect.height());
                ui.set_clip_rect(rect);
                ui.painter().rect_filled(rect, 0.0, surface);
                ui.painter().line_segment(
                    [rect.left_top(), rect.left_bottom()],
                    Stroke::new(1.0, rule),
                );
                self.grip(ui, rect, &mut output);
                let inner = rect.shrink2(egui::vec2(0.0, 0.0));
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner.shrink2(egui::vec2(10.0, 8.0)))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                output.stop_following = self.header(&mut child, context);
                child.add_space(6.0);
                let body = child.available_rect_before_wrap();
                let mut keyboard = false;
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let mut body_ui = child.new_child(egui::UiBuilder::new().max_rect(body));
                    match &mut tab.body {
                        Body::Terminal(terminal) => {
                            let restart = if let Some(session) = terminal.session.as_mut() {
                                let view = terminal.view.show(&mut body_ui, tab.id, session);
                                keyboard = view.has_focus;
                                output.toggle |= view.toggle_dock;
                                view.restart
                            } else {
                                body_ui.label(
                                    RichText::new(format!(
                                        "Could not start the terminal: {}",
                                        terminal.error.as_deref().unwrap_or("unknown error")
                                    ))
                                    .color(color32::ui_danger()),
                                );
                                body_ui.button("Try again").clicked()
                            };
                            if restart {
                                terminal.start(true);
                            }
                        }
                        Body::Chat(chat) => {
                            keyboard = chat.show(&mut body_ui, context).has_focus;
                        }
                    }
                }
                self.keyboard = keyboard;
            });
        output
    }

    fn grip(&mut self, ui: &mut egui::Ui, rect: Rect, output: &mut DockOutput) {
        let grip = Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + GRIP, rect.bottom()),
        );
        let response = ui.interact(grip, egui::Id::new("dock_grip"), egui::Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().line_segment(
                [grip.center_top(), grip.center_bottom()],
                Stroke::new(2.0, ui.visuals().selection.stroke.color),
            );
        }
        if response.drag_started() {
            self.dragging = Some(rect.width());
        }
        if let (Some(width), Some(pos)) = (self.dragging.as_mut(), response.interact_pointer_pos())
        {
            let screen = ui.ctx().content_rect().right();
            *width = (screen - pos.x).clamp(MINIMUM, MAXIMUM);
        }
        if response.drag_stopped() {
            output.resized = self.dragging.take();
        }
    }

    /// Returns whether "stop following" was pressed.
    fn header(&mut self, ui: &mut egui::Ui, context: &DockContext) -> bool {
        let mut stop = false;
        let mut close = None;
        let mut open: Option<Option<Agent>> = None;
        if !self.in_column {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (index, tab) in self.tabs.iter().enumerate() {
                    let selected = index == self.active;
                    let title = tab.title();
                    let exited = tab.exited();
                    let text = if exited {
                        RichText::new(format!("{title} ·")).weak()
                    } else {
                        RichText::new(title)
                    };
                    if ui.selectable_label(selected, text).clicked() {
                        self.active = index;
                        if let Some(tab) = self.tabs.get_mut(index) {
                            tab.focus();
                        }
                        break;
                    }
                    if selected
                        && ui
                            .small_button("×")
                            .on_hover_text("Close this tab and hang up on what runs in it")
                            .clicked()
                    {
                        close = Some(index);
                    }
                }
                if ui.small_button("+").on_hover_text("New terminal").clicked() {
                    open = Some(None);
                }
            });
        }
        let mut chat = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for agent in Agent::ALL {
                let tip = if self.agents_in_terminal {
                    format!("{} in a terminal tab, in {}", agent.label(), context.place)
                } else {
                    format!("Chat with {}, working in {}", agent.label(), context.place)
                };
                if ui.button(agent.label()).on_hover_text(tip).clicked() {
                    if self.agents_in_terminal {
                        open = Some(Some(agent));
                    } else {
                        chat = Some(agent);
                    }
                }
            }
            ui.checkbox(&mut self.agents_in_terminal, "in terminal")
                .on_hover_text(
                    "Open the agent's own interface in a terminal tab instead of a chat",
                );
        });
        ui.add(
            egui::Label::new(
                RichText::new(format!("in {}", context.place))
                    .small()
                    .weak(),
            )
            .truncate(),
        )
        .on_hover_text("New tabs start in this folder");
        if let Some(following) = &context.following {
            ui.horizontal(|ui| {
                let (text, colour) = match &following.error {
                    Some(_) => (
                        format!("following {} · last change refused", following.name),
                        color32::ui_danger(),
                    ),
                    None => (
                        format!(
                            "following {} · {}",
                            following.name,
                            match following.applied {
                                0 => "applies on save".to_string(),
                                1 => "applied once".to_string(),
                                n => format!("applied {n} times"),
                            }
                        ),
                        color32::ui_success(),
                    ),
                };
                let label =
                    ui.add(egui::Label::new(RichText::new(text).small().color(colour)).truncate());
                let hover = following.error.clone().unwrap_or_else(|| {
                    "Saving this file applies it to the preview at the playhead. \
                     It is also in $MUSIALIZER_TREATMENT for new tabs."
                        .to_string()
                });
                label.on_hover_text(hover);
                stop = ui
                    .small_button("stop")
                    .on_hover_text("Stop following; the preview keeps what was applied")
                    .clicked();
            });
        }
        if let Some(index) = close {
            self.close_tab(index);
        }
        if let Some(agent) = open {
            self.open_tab(agent, context, true);
        }
        if let Some(agent) = chat {
            self.open_chat(agent, context);
        }
        stop
    }
}

/// A terminal's state and the non-empty lines of its screen, for `dock:`.
fn terminal_state(terminal: &Terminal) -> (String, String) {
    let state = match (&terminal.session, &terminal.error) {
        (Some(session), _) => match session.exit() {
            None => "running".to_string(),
            Some(Some(code)) => format!("exited {code}"),
            Some(None) => "signalled".to_string(),
        },
        (None, Some(error)) => format!("failed: {error}"),
        (None, None) => "not started".to_string(),
    };
    let screen = terminal
        .session
        .as_ref()
        .map(terminal_view::screen_text)
        .unwrap_or_default();
    let lines: Vec<&str> = screen
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    (state, lines.join(" | "))
}

impl Drop for Dock {
    fn drop(&mut self) {
        self.shutdown();
    }
}
