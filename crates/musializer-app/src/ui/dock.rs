//! The dock: the terminals and the agents run in them (WX-1, WX-2), drawn as
//! the right column's Direct view since WX-5d (`dock/column_view.rs`; design
//! in `docs/WORKSTATION.md`).
//!
//! **Not modal**, unlike the egui Editor. The dock is drawn in the same egui
//! pass as the Editor and is built *before* it, so a key the terminal used (Escape above all, which interrupts an agent) is gone from
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
use std::sync::mpsc::{self, Receiver};

use egui::{Context, Rect};

use musializer_runtime::terminal::{GridSize, Session, Spawn};

use super::agent_choices::{self, AgentChoices};
use super::chat_store::{self, ChatSummary, Saver};
use chat_view::ChatTab;
use musializer_core::agent::SavedChat;
use terminal_view::TerminalView;

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
    /// What chats are kept under: the `.musi`, else the audio file, else the
    /// folder. Empty for a dock that keeps none.
    pub project: String,
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
    pub last_applied_at: Option<std::time::Instant>,
    pub changed_sections: Option<usize>,
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
    /// Stop following the treatment (WX-3).
    pub stop_following: bool,
}

#[derive(Default)]
pub struct Dock {
    tabs: Vec<Tab>,
    keyboard: bool,
    shells_opened: u32,
    /// Whether the agent buttons open a native chat (the default) or the
    /// agent's own TUI in a terminal tab.
    agents_in_terminal: bool,
    opened: u64,
    /// Agent replies the user has had on screen; more than this is an
    /// unseen reply, the Direct tab's ink dot.
    seen_replies: usize,
    /// A permission prompt has already posted its notice while away.
    waiting_noticed: bool,
    /// The Direct view's shown chat and terminal (WX-5d), as tab indices;
    /// a stale one falls back to the newest of its kind.
    chat_active: Option<usize>,
    terminal_active: Option<usize>,
    close_chat_confirmation: Option<egui::Id>,
    /// The Direct view's terminal pane: expanded, and its dragged height.
    terminal_open: bool,
    terminal_height: Option<f32>,
    /// The followed treatment's apply count already noted in the chat.
    applied_seen: u32,
    /// The empty state's Start buttons and discovery states as last drawn,
    /// for `dock:`; empty while a chat is shown.
    start_rects: Vec<(&'static str, egui::Rect, &'static str)>,
    /// What each agent's chips last chose (`agent_choices`).
    choices: AgentChoices,
    choices_loaded: bool,
    /// The file could not be read; it is never written over.
    choices_refused: bool,
    /// The project's saved chats, newest first, for `earlier_for`.
    earlier: Vec<ChatSummary>,
    earlier_for: Option<String>,
    earlier_loading: Option<Receiver<(String, Vec<ChatSummary>, usize)>>,
    /// A press on a row, on its way through the store's thread.
    reopening: Option<Receiver<Result<SavedChat, String>>>,
    forgetting: Option<(String, Receiver<Result<(), String>>)>,
    /// Files in the project's folder that could not be read.
    earlier_refused: usize,
    /// The chat whose × was pressed, waiting for Forget or Keep.
    forget_pending: Option<String>,
    /// Why the last reopen or forget failed.
    earlier_error: Option<String>,
    /// Where the rows were drawn, by the name `agent-press=` uses.
    earlier_rects: Vec<(String, egui::Rect)>,
    saver: Option<Saver>,
    /// A probe run keeps chats only in a place it was given (`chat_store`).
    probe_run: bool,
    chats_opened: u64,
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
        // Batch launches deliberately clear SHELL to skip rc discovery. The
        // PTY library treats an empty value as an executable name, so make
        // its terminal fallback explicit without changing normal login shells.
        let empty_shell = std::env::var_os("SHELL").is_some_and(|shell| shell.is_empty());
        let mut env = context.env.clone();
        if empty_shell {
            env.insert("SHELL".into(), program.clone());
        }
        let spawn = Spawn {
            program: agent
                .map(|agent| {
                    (
                        program.clone(),
                        vec![
                            "-i".into(),
                            "-c".into(),
                            format!("{}; exec \"$SHELL\" -i", agent.command()),
                        ],
                    )
                })
                .or_else(|| empty_shell.then(|| (program.clone(), vec!["-i".into()]))),
            working_directory: context.working_directory.clone(),
            env,
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
        self.terminal_active = Some(self.tabs.len() - 1);
    }

    fn next_id(&mut self) -> egui::Id {
        self.opened += 1;
        egui::Id::new(("dock_tab", self.opened))
    }

    /// Opens a native chat with `agent` (WX-4).
    pub fn open_chat(&mut self, agent: Agent, context: &DockContext) {
        musializer_runtime::agent_process::warm_shell_path();
        let id = self.next_id();
        let selection = self.choices().get(agent.command());
        self.chats_opened += 1;
        let chat_id = chat_store::new_id(self.chats_opened);
        let chat = ChatTab::new(agent, context, id, selection, chat_id);
        self.tabs.push(Tab {
            label: agent.label().to_string(),
            body: Body::Chat(chat),
            id,
        });
        self.chat_active = Some(self.tabs.len() - 1);
    }

    /// Runs without keeping chats unless a place is named (`chat_store`).
    pub fn set_probe_run(&mut self, probe_run: bool) {
        self.probe_run = probe_run;
    }

    /// Reads `context`'s project's chats when the project changes, off the
    /// UI thread, and takes the answer when it comes.
    pub(super) fn refresh_earlier(&mut self, context: &DockContext) {
        if self.earlier_for.as_deref() != Some(context.project.as_str()) {
            self.earlier_for = Some(context.project.clone());
            self.earlier.clear();
            self.earlier_refused = 0;
            self.forget_pending = None;
            self.earlier_loading = None;
            if let Some(root) =
                chat_store::root(self.probe_run).filter(|_| !context.project.is_empty())
            {
                let folder = chat_store::project_folder(&root, &context.project);
                let project = context.project.clone();
                let (done, answer) = mpsc::channel();
                std::thread::spawn(move || {
                    let (chats, refused) = chat_store::list(&folder, &project);
                    let _ = done.send((project, chats, refused));
                });
                self.earlier_loading = Some(answer);
            }
        }
        if let Some(answer) = &self.earlier_loading {
            if let Ok((project, chats, refused)) = answer.try_recv() {
                self.earlier_loading = None;
                if self.earlier_for.as_deref() == Some(project.as_str()) {
                    // A chat saved while the list was read is newer than it.
                    for chat in chats {
                        if !self.earlier.iter().any(|kept| kept.id == chat.id) {
                            self.earlier.push(chat);
                        }
                    }
                    self.earlier
                        .sort_by_key(|chat| std::cmp::Reverse(chat.updated_unix));
                    self.earlier_refused = refused;
                }
            }
        }
        if let Some(Ok(answer)) = self.reopening.as_ref().map(Receiver::try_recv) {
            self.reopening = None;
            self.reopened(answer, context);
        }
        if let Some((chat_id, Ok(answer))) = self
            .forgetting
            .as_ref()
            .map(|(chat_id, answer)| (chat_id.clone(), answer.try_recv()))
        {
            self.forgetting = None;
            match answer {
                Ok(()) => self.earlier.retain(|chat| chat.id != chat_id),
                Err(error) => self.earlier_error = Some(error),
            }
        }
    }

    /// Where `chat` is kept, when chats are kept.
    fn chat_file(&self, chat: &ChatTab) -> Option<PathBuf> {
        let root = chat_store::root(self.probe_run)?;
        (!chat.project.is_empty()).then(|| {
            chat_store::chat_path(
                &chat_store::project_folder(&root, &chat.project),
                &chat.chat_id,
            )
        })
    }

    /// The project's saved chats that are not open in a tab.
    pub(super) fn earlier_closed(&self) -> Vec<ChatSummary> {
        self.earlier
            .iter()
            .filter(|chat| {
                !self
                    .tabs
                    .iter()
                    .any(|tab| matches!(&tab.body, Body::Chat(open) if open.chat_id == chat.id))
            })
            .cloned()
            .collect()
    }

    /// Opens a saved chat, or shows it if it is already open.
    pub(super) fn reopen(&mut self, chat_id: &str) {
        self.earlier_error = None;
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| matches!(&tab.body, Body::Chat(open) if open.chat_id == chat_id))
        {
            self.chat_active = Some(index);
            return;
        }
        if self.reopening.is_some() {
            return;
        }
        let Some(path) = self
            .earlier
            .iter()
            .find(|chat| chat.id == chat_id)
            .map(|chat| chat.path.clone())
        else {
            return;
        };
        let project = self.earlier_for.clone().unwrap_or_default();
        let saver = self.saver.get_or_insert_with(Saver::start);
        self.reopening = Some(saver.load(path, chat_id.to_owned(), project));
    }

    /// The store's answer to a reopen, which holds the chat's lock.
    fn reopened(&mut self, answer: Result<SavedChat, String>, context: &DockContext) {
        let saved = match answer {
            Ok(saved) => saved,
            Err(error) => {
                self.earlier_error = Some(error);
                return;
            }
        };
        musializer_runtime::agent_process::warm_shell_path();
        let id = self.next_id();
        match ChatTab::reopen(&saved, context, id) {
            Some(chat) => {
                self.tabs.push(Tab {
                    label: chat.agent.label().to_string(),
                    body: Body::Chat(chat),
                    id,
                });
                self.chat_active = Some(self.tabs.len() - 1);
            }
            None => {
                self.earlier_error = Some("That chat could not be read.".to_owned());
                let path = self.earlier.iter().find(|chat| chat.id == saved.id);
                if let (Some(chat), Some(saver)) = (path, &self.saver) {
                    saver.release(chat.path.clone());
                }
            }
        }
    }

    /// Removes a saved chat from this project, and only from here: the
    /// agent's own history is its CLI's.
    pub(super) fn forget(&mut self, chat_id: &str) {
        self.forget_pending = None;
        if self.forgetting.is_some() {
            return;
        }
        let Some(path) = self
            .earlier
            .iter()
            .find(|chat| chat.id == chat_id)
            .map(|chat| chat.path.clone())
        else {
            return;
        };
        let saver = self.saver.get_or_insert_with(Saver::start);
        self.forgetting = Some((chat_id.to_owned(), saver.forget(path)));
    }

    /// Writes the chats that want writing, without stalling the frame.
    fn save_chats(&mut self) {
        let root = chat_store::root(self.probe_run);
        for tab in &mut self.tabs {
            let Body::Chat(chat) = &mut tab.body else {
                continue;
            };
            if !chat.take_save_due() {
                continue;
            }
            let (Some(root), Some(saved)) = (root.as_ref(), chat.saved()) else {
                continue;
            };
            if saved.project.is_empty() {
                continue;
            }
            let Ok(bytes) = serde_json::to_vec_pretty(&saved) else {
                continue;
            };
            if bytes.len() > chat_store::MAX_CHAT_BYTES {
                chat.note_too_long();
                continue;
            }
            let folder = chat_store::project_folder(root, &saved.project);
            let path = chat_store::chat_path(&folder, &saved.id);
            self.saver
                .get_or_insert_with(Saver::start)
                .save(path.clone(), bytes);
            if self.earlier_for.as_deref() == Some(saved.project.as_str()) {
                self.earlier.retain(|kept| kept.id != saved.id);
                self.earlier.insert(0, ChatSummary::of(&saved, path));
            }
        }
        if let Some(failure) = self.saver.as_ref().and_then(Saver::failure) {
            self.earlier_error = Some("A chat could not be saved.".to_owned());
            eprintln!("chat store: {failure}");
        }
    }

    /// What each agent last chose, read once. A file that cannot be read
    /// is left alone: chats run on defaults and nothing writes over it.
    fn choices(&mut self) -> &AgentChoices {
        if !self.choices_loaded {
            self.choices_loaded = true;
            match agent_choices::default_path().map(|path| agent_choices::load(&path)) {
                Some(Ok(Some(choices))) => self.choices = choices,
                Some(Ok(None)) => {}
                None => self.choices_refused = true,
                Some(Err(error)) => {
                    self.choices_refused = true;
                    eprintln!("agent choices: {error}; not saving over them");
                }
            }
        }
        &self.choices
    }

    /// Keeps a chat's changed chips for the next chat with that agent.
    fn save_choices(&mut self) {
        let mut changed = false;
        let changes: Vec<_> = self
            .tabs
            .iter_mut()
            .filter_map(|tab| match &mut tab.body {
                Body::Chat(chat) => chat
                    .take_selection_change()
                    .map(|selection| (chat.agent, selection)),
                Body::Terminal(_) => None,
            })
            .collect();
        self.choices();
        for (agent, selection) in changes {
            self.choices.remember(agent.command(), &selection);
            changed = true;
        }
        if !changed || self.choices_refused {
            return;
        }
        if let Some(path) = agent_choices::default_path() {
            if let Err(error) = agent_choices::save(&path, &self.choices) {
                eprintln!("agent choices: {error}");
            }
        }
    }

    /// A control of the shown chat, by the name the `dock:` line gives it
    /// (`--ui-probe agent-press=`).
    #[must_use]
    pub fn probe_point(&self, name: &str) -> Option<egui::Pos2> {
        if let Some((_, rect)) = self.earlier_rects.iter().find(|(row, _)| row == name) {
            return Some(rect.center());
        }
        let index = self.current_chat()?;
        let Body::Chat(chat) = &self.tabs.get(index)?.body else {
            return None;
        };
        chat.layout
            .iter()
            .find(|(kind, _)| *kind == name)
            .map(|(_, rect)| rect)
            .or_else(|| {
                let row = name.strip_prefix("row:")?;
                chat.menu_rows
                    .iter()
                    .find(|(id, _)| id == row)
                    .map(|(_, rect)| rect)
            })
            .map(|rect| rect.center())
    }

    /// Sends `text` to the active chat as if typed (`--ui-probe agent-say=`).
    pub fn probe_say(&mut self, text: &str, context: &DockContext) {
        let index = self.current_chat().unwrap_or(usize::MAX);
        if let Some(Body::Chat(chat)) = self.tabs.get_mut(index).map(|tab| &mut tab.body) {
            chat.probe_say(text, context);
        }
    }

    /// Answers the active chat's open permission prompts
    /// (`--ui-probe agent-decide=`).
    pub fn probe_decide(&mut self, allow: bool) {
        let index = self.current_chat().unwrap_or(usize::MAX);
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
        self.save_choices();
        self.save_chats();
    }

    /// Types `text` into the active terminal (`--ui-probe dock-run=`). Xvfb
    /// has no keyboard; key encoding is covered by `keys`' own tests.
    pub fn probe_write(&mut self, text: &str) {
        let index = self.current_terminal().unwrap_or(usize::MAX);
        if let Some(Body::Terminal(Terminal {
            session: Some(session),
            ..
        })) = self.tabs.get(index).map(|tab| &tab.body)
        {
            session.write(text.as_bytes().to_vec());
        }
    }

    /// Asks the active terminal to take the keyboard on the next frame.
    pub fn focus_active(&mut self) {
        // The pane's terminal when it is open, else the chat's composer: what
        // Ctrl+` lands on is what the user would type into.
        let index = if self.terminal_open {
            self.current_terminal()
        } else {
            self.current_chat()
        };
        if let Some(tab) = index.and_then(|index| self.tabs.get_mut(index)) {
            tab.focus();
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            if let Body::Chat(chat) = &mut self.tabs[index].body {
                chat.mark_save_due();
            }
            self.save_chats();
            let mut tab = self.tabs.remove(index);
            if let Body::Chat(chat) = &tab.body {
                if let (Some(path), Some(saver)) = (self.chat_file(chat), &self.saver) {
                    saver.release(path);
                }
            }
            tab.close();
            // Both selections are indices into the same mixed tab list.
            for active in [&mut self.chat_active, &mut self.terminal_active] {
                *active = active.and_then(|selected| {
                    if selected == index {
                        None
                    } else {
                        Some(selected - usize::from(selected > index))
                    }
                });
            }
            if self.close_chat_confirmation == Some(tab.id) {
                self.close_chat_confirmation = None;
            }
        }
    }

    /// Hangs up on every terminal. Called once, before the window closes.
    pub fn shutdown(&mut self) {
        while !self.tabs.is_empty() {
            self.close_tab(0);
        }
        // The window closing is the last chance for these to land.
        if let Some(saver) = self.saver.as_mut() {
            saver.finish();
        }
    }

    /// One line for the run report: the tabs, which is active, and the first
    /// non-empty line of its screen, so a capture's terminal content can be
    /// asserted without reading pixels.
    #[must_use]
    pub fn describe(&self, open: bool) -> String {
        self.describe_column(open)
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

    /// Builds the dock into `rect` (egui points): the column's Direct view.
    pub fn show(&mut self, ctx: &Context, rect: Rect, context: &DockContext) -> DockOutput {
        self.show_column(ctx, rect, context)
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
