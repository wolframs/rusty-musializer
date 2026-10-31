//! A conversation with a coding agent CLI, as the dock's chat tab shows it
//! (WX-4; design in `docs/WORKSTATION.md`).
//!
//! Three CLIs, three wire protocols, one transcript:
//!
//! | Agent | Process | Protocol |
//! | --- | --- | --- |
//! | Claude Code | `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio` | NDJSON `type`-tagged events; permission prompts arrive as `control_request` and are answered with `control_response`; models, effort and permission mode change live over the same channel |
//! | Antigravity | `agy --input-format stream-json --output-format stream-json --print=` | NDJSON `event`-tagged `step_update`s; it cannot ask for a permission in this mode, so a denied action is reported in its `result` |
//! | Codex | `codex app-server` | JSON-RPC over stdio: `initialize`, `model/list`, `thread/start`, `turn/start`, which carries the model, effort and access of that turn; approvals arrive as server requests |
//!
//! The shapes were read off live sessions of each CLI on 2026-10-21 (Claude
//! Code 2.1.301, codex-cli 0.163.0, the agy build of that day), and the tests
//! below replay synthetic lines of the same shape. Nothing here knows about a
//! process. [`Transcript::receive`] takes one line the agent printed and returns
//! the lines to write back, which is how a permission answer or a JSON-RPC
//! handshake gets out. The runtime moves bytes and the app draws.
//!
//! Unknown events are ignored rather than refused. These protocols grow every
//! week, and a chat that stops at the first field it has not seen would be
//! broken by the next CLI update. What *is* refused is a line that is not JSON
//! at all: it is shown as a notice, because an agent printing prose to stdout
//! has been started wrong.
//!
//! **What the composer picks.** A [`Selection`] is a model, an effort and an
//! [`Access`]. Claude and Codex list their own models over the protocol, so the
//! picker offers what this account can run rather than a list that ages here.
//! Antigravity lists them from `agy models`, which the app runs and hands to
//! [`antigravity_models`]. A choice reaches each CLI differently, as far as its
//! protocol allows: Claude takes it live, Codex with the next turn, and
//! Antigravity only on a new process, so its choices raise
//! [`Transcript::take_restart_owed`]. The access names and their mapping onto
//! each CLI's modes follow T3 Code (MIT, credited in the About window).

use std::collections::HashMap;

use serde_json::{json, Value};

/// Which wire protocol a transcript speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Claude,
    Antigravity,
    Codex,
}

/// How much an agent may do without asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Access {
    Supervised,
    /// What a chat has always started in: the CLI's edit-accepting mode.
    #[default]
    AcceptEdits,
    Auto,
    Full,
}

impl Access {
    pub const ALL: [Self; 4] = [Self::Supervised, Self::AcceptEdits, Self::Auto, Self::Full];

    /// The name in a preference file and on a probe line.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::AcceptEdits => "accept-edits",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }

    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|access| access.key() == key)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Supervised => "Supervised",
            Self::AcceptEdits => "Auto-accept edits",
            Self::Auto => "Auto",
            Self::Full => "Full access",
        }
    }

    /// The name on the composer's chip, where width is short.
    #[must_use]
    pub const fn chip(self) -> &'static str {
        match self {
            Self::AcceptEdits => "Edits",
            other => other.label(),
        }
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Supervised => "Ask before commands and file changes.",
            Self::AcceptEdits => "Auto-approve edits, ask before other actions.",
            Self::Auto => "The agent approves routine actions and asks about the rest.",
            Self::Full => "Allow commands and edits without prompts.",
        }
    }

    const fn claude_mode(self) -> &'static str {
        match self {
            Self::Supervised => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::Full => "bypassPermissions",
        }
    }

    const fn codex_approval(self) -> &'static str {
        match self {
            Self::Supervised => "untrusted",
            Self::AcceptEdits | Self::Auto => "on-request",
            Self::Full => "never",
        }
    }

    /// Named on every thread and turn, because the user's own Codex config
    /// may route approvals to its reviewer agent, and then Auto-accept edits
    /// would quietly mean Auto.
    const fn codex_reviewer(self) -> &'static str {
        match self {
            Self::Auto => "auto_review",
            _ => "user",
        }
    }

    const fn codex_sandbox_mode(self) -> &'static str {
        match self {
            Self::Supervised => "read-only",
            Self::AcceptEdits | Self::Auto => "workspace-write",
            Self::Full => "danger-full-access",
        }
    }

    fn codex_sandbox_policy(self) -> Value {
        match self {
            Self::Supervised => json!({"type": "readOnly"}),
            Self::AcceptEdits | Self::Auto => json!({"type": "workspaceWrite"}),
            Self::Full => json!({"type": "dangerFullAccess"}),
        }
    }
}

/// One entry of a model picker, as the CLI listed it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelChoice {
    /// What the CLI is asked for. For Antigravity, the id without its effort
    /// suffix: `gemini-3.8-flash` stands for its `-low`, `-medium` and `-high`.
    pub id: String,
    /// The full model name behind an alias (`opus` is `claude-opus-5-5`).
    pub resolved: String,
    pub label: String,
    pub description: String,
    /// In the CLI's order. Empty: this model takes no effort.
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    /// Whether [`Access::Auto`] works with this model.
    pub auto: bool,
}

impl ModelChoice {
    fn answers_to(&self, id: &str) -> bool {
        id == self.id
            || id == self.resolved
            || self
                .efforts
                .iter()
                .any(|effort| id.strip_suffix(effort.as_str()) == Some(&format!("{}-", self.id)))
    }
}

/// What the composer's three chips say. `None` leaves the choice to the CLI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: Access,
}

/// Effort levels, lowest first. `agy models` writes the level into the id.
const EFFORT_LEVELS: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];

/// An effort level as a chip says it.
#[must_use]
pub fn effort_title(level: &str) -> String {
    match level {
        "xhigh" => "XHigh".to_owned(),
        _ => {
            let mut letters = level.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        }
    }
}

/// `agy models`: one `id<TAB>label` line per model and effort, folded to one
/// entry per model. Lines without a tab are its progress text.
#[must_use]
pub fn antigravity_models(listing: &str) -> Vec<ModelChoice> {
    let mut models: Vec<ModelChoice> = Vec::new();
    for line in listing.lines() {
        let Some((id, label)) = line.trim().split_once('\t') else {
            continue;
        };
        let (family, level) = EFFORT_LEVELS
            .iter()
            .find_map(|level| {
                id.strip_suffix(level)?
                    .strip_suffix('-')
                    .map(|family| (family, *level))
            })
            .unwrap_or((id, ""));
        let label = label.rsplit_once(" (").map_or(label, |(name, _)| name);
        let index = models
            .iter()
            .position(|model| model.id == family)
            .unwrap_or_else(|| {
                models.push(ModelChoice {
                    id: family.to_owned(),
                    resolved: family.to_owned(),
                    label: label.to_owned(),
                    auto: true,
                    ..ModelChoice::default()
                });
                models.len() - 1
            });
        if !level.is_empty() {
            models[index].efforts.push(level.to_owned());
        }
    }
    for model in &mut models {
        model.efforts.sort_by_key(|effort| {
            EFFORT_LEVELS
                .iter()
                .position(|level| level == effort)
                .unwrap_or(EFFORT_LEVELS.len())
        });
        model.default_effort = model
            .efforts
            .iter()
            .find(|effort| *effort == "medium")
            .or(model.efforts.first())
            .cloned();
    }
    models
}

/// Where a tool call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Done,
    Failed,
}

/// How a permission prompt is answered on the wire.
#[derive(Clone, Debug, PartialEq)]
pub enum PermissionKey {
    /// Claude's `control_request` id, and the tool input to echo back on allow.
    Claude { request_id: String, input: Value },
    /// A Codex server request's JSON-RPC id.
    Codex { rpc_id: Value },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        /// One line: the command, the path, the query.
        detail: String,
        state: ToolState,
        /// The first part of what it returned.
        output: String,
    },
    Permission {
        key: PermissionKey,
        tool: String,
        detail: String,
        /// `None` until answered.
        allowed: Option<bool>,
    },
    Notice {
        text: String,
        error: bool,
    },
    /// A picture on disk the agent produced or pointed at. The file is the
    /// picture: a transcript holds its path, never its pixels.
    Image {
        path: String,
    },
}

/// How much of a tool's output a transcript keeps. The chat shows a preview;
/// the whole output stays in the agent's own context.
pub const OUTPUT_PREVIEW: usize = 2_000;

#[derive(Debug, Default)]
struct CodexState {
    next_id: u64,
    pending: HashMap<u64, CodexCall>,
    thread: Option<String>,
    turn: Option<String>,
    queued: Vec<String>,
    cwd: String,
    instructions: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexCall {
    Initialize,
    ModelList,
    ThreadStart,
    TurnStart,
    Other,
}

#[derive(Debug, Default)]
struct ClaudeState {
    next_id: u64,
    pending: HashMap<String, ClaudeCall>,
}

/// A control request of ours, and what to put back if Claude refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClaudeCall {
    Initialize,
    Settings,
    SetModel(Option<String>),
    SetEffort(Option<String>),
    SetAccess(Access),
}

/// One conversation.
#[derive(Debug)]
pub struct Transcript {
    pub protocol: Protocol,
    pub entries: Vec<Entry>,
    /// A turn is running.
    pub busy: bool,
    pub model: Option<String>,
    /// The agent's own session or conversation id, for resuming it.
    pub session: Option<String>,
    /// The last turn's cost or token count, as one short phrase.
    pub usage: Option<String>,
    /// What the composer has chosen.
    pub selection: Selection,
    /// What the CLI offers; empty until it has listed them.
    pub models: Vec<ModelChoice>,
    /// Why the list is missing, when it could not be read.
    pub models_error: Option<String>,
    /// The effort the CLI says it is using.
    pub applied_effort: Option<String>,
    /// The permission mode the CLI last said it is in, in its own word.
    pub reported_mode: Option<String>,
    /// A refusal put a choice back; what was saved is out of date.
    pub selection_reverted: bool,
    /// Antigravity said this account has no auto mode.
    auto_unavailable: bool,
    /// A choice that only a new process can take.
    restart_owed: bool,
    /// Text being streamed, keyed by the protocol's own handle for it, so the
    /// final message replaces the streamed text rather than repeating it.
    open_text: HashMap<String, usize>,
    /// Tool entries by the protocol's id.
    tools: HashMap<String, usize>,
    /// Claude's id for the message being streamed.
    message: Option<String>,
    codex: CodexState,
    claude: ClaudeState,
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// One line of a tool's input, for the chip.
fn tool_detail(input: &Value) -> String {
    for key in [
        "command",
        "CommandLine",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ] {
        if let Some(text) = input.get(key).and_then(Value::as_str) {
            return truncate(text.lines().next().unwrap_or(""), 160);
        }
    }
    match input {
        Value::Object(map) if map.is_empty() => String::new(),
        Value::Null => String::new(),
        other => truncate(&other.to_string(), 160),
    }
}

/// A tool result's content as text: a string, or an array of text blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// One entry of Claude's `initialize` reply.
fn claude_model(model: &Value) -> ModelChoice {
    let text = |key: &str| model[key].as_str().unwrap_or("").to_owned();
    ModelChoice {
        id: text("value"),
        resolved: text("resolvedModel"),
        label: text("displayName"),
        description: text("description"),
        efforts: model["supportedEffortLevels"]
            .as_array()
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        default_effort: None,
        auto: model["supportsAutoMode"].as_bool().unwrap_or(false),
    }
}

/// One entry of Codex's `model/list`.
fn codex_model(model: &Value) -> ModelChoice {
    let text = |key: &str| model[key].as_str().unwrap_or("").to_owned();
    ModelChoice {
        id: text("id"),
        resolved: text("model"),
        label: text("displayName"),
        description: text("description"),
        efforts: model["supportedReasoningEfforts"]
            .as_array()
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| level["reasoningEffort"].as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        default_effort: model["defaultReasoningEffort"].as_str().map(str::to_owned),
        auto: true,
    }
}

impl Transcript {
    #[must_use]
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            entries: Vec::new(),
            busy: false,
            model: None,
            session: None,
            usage: None,
            selection: Selection::default(),
            models: Vec::new(),
            models_error: None,
            applied_effort: None,
            reported_mode: None,
            selection_reverted: false,
            auto_unavailable: false,
            restart_owed: false,
            open_text: HashMap::new(),
            tools: HashMap::new(),
            message: None,
            codex: CodexState::default(),
            claude: ClaudeState::default(),
        }
    }

    /// The flags that carry the selection into a new process. Codex takes
    /// nothing here: its thread and turn requests carry it.
    #[must_use]
    pub fn launch_flags(&self) -> Vec<String> {
        let mut flags: Vec<String> = Vec::new();
        match self.protocol {
            Protocol::Claude => {
                flags.push("--permission-mode".into());
                flags.push(self.selection.access.claude_mode().into());
                // Only makes Full access reachable from a running session;
                // the mode above is still what applies.
                flags.push("--allow-dangerously-skip-permissions".into());
                if let Some(model) = &self.selection.model {
                    flags.push("--model".into());
                    flags.push(model.clone());
                }
                if let Some(effort) = &self.selection.effort {
                    flags.push("--effort".into());
                    flags.push(effort.clone());
                }
            }
            Protocol::Antigravity => {
                flags.push("--mode".into());
                flags.push(
                    match self.selection.access {
                        Access::Auto => "auto",
                        _ => "accept-edits",
                    }
                    .into(),
                );
                if self.selection.access == Access::Full {
                    flags.push("--dangerously-skip-permissions".into());
                }
                if let Some(model) = &self.selection.model {
                    flags.push("--model".into());
                    flags.push(match &self.selection.effort {
                        Some(effort) => format!("{model}-{effort}"),
                        None => model.clone(),
                    });
                }
            }
            Protocol::Codex => {}
        }
        flags
    }

    /// The listed model the chat is on: the chosen one, else the one the CLI
    /// reports. An alias answers for its model, so `default` never wins over
    /// the name of what it stands for.
    #[must_use]
    pub fn current_model(&self) -> Option<&ModelChoice> {
        let id = self.selection.model.as_deref().or(self.model.as_deref())?;
        let listed = self.models.iter().find(|model| model.answers_to(id))?;
        self.models
            .iter()
            .find(|model| model.id != "default" && model.resolved == listed.resolved)
            .or(Some(listed))
    }

    /// The model chip's words.
    #[must_use]
    pub fn model_label(&self) -> String {
        self.current_model().map_or_else(
            || {
                self.selection
                    .model
                    .clone()
                    .or_else(|| self.model.clone())
                    .unwrap_or_else(|| "Default".to_owned())
            },
            |model| model.label.clone(),
        )
    }

    /// The level the chat is on. Claude's word for it wins over the choice,
    /// because it can apply less than it was asked for. Otherwise: chosen,
    /// else reported, else the model's own.
    #[must_use]
    pub fn current_effort(&self) -> Option<String> {
        let reported = self.applied_effort.clone();
        if self.protocol == Protocol::Claude && reported.is_some() {
            return reported;
        }
        self.selection
            .effort
            .clone()
            .or(reported)
            .or_else(|| self.current_model()?.default_effort.clone())
    }

    /// The effort chip's words.
    #[must_use]
    pub fn effort_label(&self) -> String {
        self.current_effort()
            .map_or_else(|| "Default".to_owned(), |effort| effort_title(&effort))
    }

    /// Why this chat cannot use `access`, in words for the menu.
    #[must_use]
    pub fn access_refusal(&self, access: Access) -> Option<String> {
        match (self.protocol, access) {
            (Protocol::Antigravity, Access::Supervised) => {
                Some("Antigravity cannot ask from a chat.".to_owned())
            }
            (Protocol::Antigravity, Access::Auto) if self.auto_unavailable => {
                Some("This Antigravity account has no auto mode.".to_owned())
            }
            (Protocol::Claude, Access::Auto) => self
                .current_model()
                .filter(|model| !model.auto)
                .map(|model| format!("{} has no auto mode.", model.label)),
            _ => None,
        }
    }

    /// A line the agent wrote to stderr. Antigravity's only word that it did
    /// not take `--mode auto` is a warning there, and it carries on in its
    /// default mode. Unmatched wording only loses the explanation.
    pub fn diagnostic(&mut self, line: &str) {
        if self.protocol == Protocol::Antigravity
            && line.contains("--mode auto is not available")
            && !self.auto_unavailable
        {
            self.auto_unavailable = true;
            self.selection.access = Access::default();
            self.selection_reverted = true;
            self.notice(
                "Antigravity has no auto mode on this account, so it stays on Auto-accept edits.",
                true,
            );
        }
    }

    /// Whether a choice waits for a new process. Asking clears it.
    pub fn take_restart_owed(&mut self) -> bool {
        std::mem::take(&mut self.restart_owed)
    }

    /// Whether a refusal has put a choice back since the last time of asking.
    pub fn take_selection_reverted(&mut self) -> bool {
        std::mem::take(&mut self.selection_reverted)
    }

    /// Chooses a model. Returns the lines to write.
    pub fn choose_model(&mut self, id: &str) -> Vec<String> {
        let previous = self.selection.model.replace(id.to_owned());
        let listed = self.models.iter().find(|model| model.id == id).cloned();
        let mut out = Vec::new();
        if let Some(model) = &listed {
            // A level this model does not have would fail the next turn.
            if self
                .selection
                .effort
                .as_ref()
                .is_some_and(|effort| !model.efforts.contains(effort))
            {
                self.selection.effort = None;
            }
            if self.protocol == Protocol::Antigravity && self.selection.effort.is_none() {
                self.selection.effort.clone_from(&model.default_effort);
            }
            if self.protocol == Protocol::Claude
                && self.selection.access == Access::Auto
                && !model.auto
            {
                out = self.choose_access(Access::AcceptEdits);
            }
        }
        match self.protocol {
            Protocol::Claude => out.insert(
                0,
                self.claude_control(
                    "set_model",
                    json!({"model": id}),
                    ClaudeCall::SetModel(previous),
                ),
            ),
            // The thread's own level belongs to the model it started on.
            Protocol::Codex => self.applied_effort = None,
            Protocol::Antigravity => self.restart_owed = true,
        }
        out
    }

    /// Chooses an effort level. Returns the lines to write.
    pub fn choose_effort(&mut self, effort: &str) -> Vec<String> {
        let previous = self.selection.effort.replace(effort.to_owned());
        match self.protocol {
            Protocol::Claude => vec![self.claude_control(
                "apply_flag_settings",
                json!({"settings": {"effortLevel": effort}}),
                ClaudeCall::SetEffort(previous),
            )],
            Protocol::Codex => Vec::new(),
            Protocol::Antigravity => {
                // The model is in the flag; with the CLI's default model
                // there is nothing to attach the level to.
                self.restart_owed = true;
                if self.selection.model.is_none() {
                    self.selection.model = self.current_model().map(|model| model.id.clone());
                }
                Vec::new()
            }
        }
    }

    /// Chooses an access level. Returns the lines to write.
    pub fn choose_access(&mut self, access: Access) -> Vec<String> {
        let previous = std::mem::replace(&mut self.selection.access, access);
        match self.protocol {
            Protocol::Claude => vec![self.claude_control(
                "set_permission_mode",
                json!({"mode": access.claude_mode()}),
                ClaudeCall::SetAccess(previous),
            )],
            Protocol::Codex => Vec::new(),
            Protocol::Antigravity => {
                self.restart_owed = true;
                Vec::new()
            }
        }
    }

    /// Takes a model list from outside the protocol (`agy models`). Returns
    /// the lines to write.
    pub fn set_models(&mut self, models: Vec<ModelChoice>) -> Vec<String> {
        self.models = models;
        self.reconcile_selection()
    }

    /// A remembered model the CLI no longer offers must not fail the first
    /// turn. It says so and goes back to the CLI's default.
    fn reconcile_selection(&mut self) -> Vec<String> {
        let Some(id) = self.selection.model.clone() else {
            return Vec::new();
        };
        if self.models.is_empty() || self.models.iter().any(|model| model.answers_to(&id)) {
            return Vec::new();
        }
        self.selection.model = None;
        self.selection.effort = None;
        self.selection_reverted = true;
        self.notice(
            format!("{id} is no longer offered, so this chat uses the default model."),
            false,
        );
        match self.protocol {
            Protocol::Claude => vec![self.claude_control(
                "set_model",
                json!({"model": "default"}),
                ClaudeCall::SetModel(None),
            )],
            Protocol::Codex => Vec::new(),
            Protocol::Antigravity => {
                self.restart_owed = true;
                Vec::new()
            }
        }
    }

    fn claude_control(&mut self, subtype: &str, fields: Value, call: ClaudeCall) -> String {
        self.claude.next_id += 1;
        let request_id = format!("musializer-{}", self.claude.next_id);
        self.claude.pending.insert(request_id.clone(), call);
        let mut request = json!({"subtype": subtype});
        if let (Some(request), Value::Object(fields)) = (request.as_object_mut(), fields) {
            request.extend(fields);
        }
        json!({"type": "control_request", "request_id": request_id, "request": request}).to_string()
    }

    fn claude_settings(&mut self) -> String {
        self.claude_control("get_settings", json!({}), ClaudeCall::Settings)
    }

    fn claude_response(&mut self, response: &Value) -> Vec<String> {
        let id = response["request_id"].as_str().unwrap_or("");
        let Some(call) = self.claude.pending.remove(id) else {
            return Vec::new();
        };
        if response["subtype"].as_str() == Some("error") {
            let message = response["error"].as_str().unwrap_or("refused").to_owned();
            match call {
                ClaudeCall::Initialize => self.models_error = Some(truncate(&message, 200)),
                ClaudeCall::Settings => {}
                ClaudeCall::SetModel(previous) => {
                    self.selection.model = previous;
                    self.selection_reverted = true;
                }
                ClaudeCall::SetEffort(previous) => {
                    self.selection.effort = previous;
                    self.selection_reverted = true;
                }
                ClaudeCall::SetAccess(previous) => {
                    self.selection.access = previous;
                    self.selection_reverted = true;
                }
            }
            self.notice(format!("Claude: {}", truncate(&message, 400)), true);
            return Vec::new();
        }
        let body = &response["response"];
        match call {
            ClaudeCall::Initialize => {
                self.models = body["models"]
                    .as_array()
                    .map(|models| models.iter().map(claude_model).collect())
                    .unwrap_or_default();
                self.reported_mode = body["current_permission_mode"].as_str().map(str::to_owned);
                self.reconcile_selection()
            }
            ClaudeCall::Settings => {
                let applied = &body["applied"];
                if let Some(model) = applied["model"].as_str() {
                    self.model = Some(model.to_owned());
                }
                self.applied_effort = applied["effort"].as_str().map(str::to_owned);
                Vec::new()
            }
            ClaudeCall::SetModel(_) => {
                if let Some(model) = body["model"].as_str() {
                    self.model = Some(model.to_owned());
                }
                vec![self.claude_settings()]
            }
            ClaudeCall::SetEffort(_) => vec![self.claude_settings()],
            ClaudeCall::SetAccess(_) => {
                self.reported_mode = body["mode"].as_str().map(str::to_owned);
                Vec::new()
            }
        }
    }

    fn notice(&mut self, text: impl Into<String>, error: bool) {
        self.entries.push(Entry::Notice {
            text: text.into(),
            error,
        });
    }

    fn append_text(&mut self, key: &str, delta: &str) {
        if delta.is_empty() {
            return;
        }
        if let Some(&index) = self.open_text.get(key) {
            if let Some(Entry::Assistant { text }) = self.entries.get_mut(index) {
                text.push_str(delta);
                return;
            }
        }
        self.entries.push(Entry::Assistant {
            text: delta.to_string(),
        });
        self.open_text
            .insert(key.to_string(), self.entries.len() - 1);
    }

    fn set_text(&mut self, key: &str, full: &str) {
        if let Some(&index) = self.open_text.get(key) {
            if let Some(Entry::Assistant { text }) = self.entries.get_mut(index) {
                full.clone_into(text);
                return;
            }
        }
        if !full.trim().is_empty() {
            self.entries.push(Entry::Assistant {
                text: full.to_string(),
            });
            self.open_text
                .insert(key.to_string(), self.entries.len() - 1);
        }
    }

    fn start_tool(&mut self, id: &str, name: &str, detail: String) {
        if self.tools.contains_key(id) {
            return;
        }
        self.entries.push(Entry::Tool {
            id: id.to_string(),
            name: name.to_string(),
            detail,
            state: ToolState::Running,
            output: String::new(),
        });
        self.tools.insert(id.to_string(), self.entries.len() - 1);
    }

    fn finish_tool(&mut self, id: &str, failed: bool, result: &str) {
        if let Some(&index) = self.tools.get(id) {
            if let Some(Entry::Tool { state, output, .. }) = self.entries.get_mut(index) {
                *state = if failed {
                    ToolState::Failed
                } else {
                    ToolState::Done
                };
                *output = truncate(result, OUTPUT_PREVIEW);
            }
        }
    }

    /// Lines to write when the process has just started.
    ///
    /// Codex needs a handshake before it takes a message; the NDJSON agents
    /// need nothing. `instructions` is the directing context: Claude gets it
    /// on its command line instead (`--append-system-prompt`), Codex as the
    /// thread's developer instructions, and Antigravity, which takes neither,
    /// with the first message.
    pub fn start(&mut self, cwd: &str, instructions: &str) -> Vec<String> {
        self.codex.cwd = cwd.to_string();
        self.codex.instructions = instructions.to_string();
        match self.protocol {
            Protocol::Codex => {
                self.busy = true;
                self.models.clear();
                vec![self.codex_request(
                    "initialize",
                    json!({"clientInfo": {"name": "musializer", "title": "Musializer", "version": env!("CARGO_PKG_VERSION")}}),
                    CodexCall::Initialize,
                )]
            }
            Protocol::Claude => vec![
                self.claude_control("initialize", json!({}), ClaudeCall::Initialize),
                self.claude_settings(),
            ],
            Protocol::Antigravity => Vec::new(),
        }
    }

    fn codex_request(&mut self, method: &str, params: Value, call: CodexCall) -> String {
        self.codex.next_id += 1;
        let id = self.codex.next_id;
        self.codex.pending.insert(id, call);
        json!({"id": id, "method": method, "params": params}).to_string()
    }

    fn codex_turn(&mut self, text: &str) -> String {
        let thread = self.codex.thread.clone().unwrap_or_default();
        let access = self.selection.access;
        let mut params = json!({
            "threadId": thread,
            "input": [{"type": "text", "text": text}],
            "approvalPolicy": access.codex_approval(),
            "approvalsReviewer": access.codex_reviewer(),
            "sandboxPolicy": access.codex_sandbox_policy(),
        });
        if let Some(model) = self.selection.model.clone() {
            params["model"] = json!(model);
            self.model = Some(model);
        }
        if let Some(effort) = &self.selection.effort {
            params["effort"] = json!(effort);
        }
        self.codex_request("turn/start", params, CodexCall::TurnStart)
    }

    /// The user says `text`. Returns the lines to write.
    pub fn send(&mut self, text: &str, first_message_preamble: Option<&str>) -> Vec<String> {
        self.entries.push(Entry::User {
            text: text.to_string(),
        });
        self.busy = true;
        let wire = match first_message_preamble {
            Some(preamble) if !preamble.is_empty() => format!("{preamble}\n\n{text}"),
            _ => text.to_string(),
        };
        match self.protocol {
            Protocol::Claude => vec![json!({
                "type": "user",
                "message": {"role": "user", "content": [{"type": "text", "text": wire}]}
            })
            .to_string()],
            Protocol::Antigravity => {
                vec![json!({"event": "user", "message": {"content": wire}}).to_string()]
            }
            Protocol::Codex => {
                if self.codex.thread.is_some() {
                    vec![self.codex_turn(&wire)]
                } else {
                    self.codex.queued.push(wire);
                    Vec::new()
                }
            }
        }
    }

    /// Stop the running turn. Antigravity has no interrupt in this mode; the
    /// caller ends its process instead.
    pub fn interrupt(&mut self) -> Vec<String> {
        match self.protocol {
            Protocol::Claude => vec![json!({
                "type": "control_request",
                "request_id": format!("musializer-interrupt-{}", self.entries.len()),
                "request": {"subtype": "interrupt"}
            })
            .to_string()],
            Protocol::Codex => match (self.codex.thread.clone(), self.codex.turn.clone()) {
                (Some(thread), Some(turn)) => vec![self.codex_request(
                    "turn/interrupt",
                    json!({"threadId": thread, "turnId": turn}),
                    CodexCall::Other,
                )],
                _ => Vec::new(),
            },
            Protocol::Antigravity => Vec::new(),
        }
    }

    /// Answers the permission prompt at `index`.
    pub fn decide(&mut self, index: usize, allow: bool) -> Vec<String> {
        let Some(Entry::Permission { key, allowed, .. }) = self.entries.get_mut(index) else {
            return Vec::new();
        };
        if allowed.is_some() {
            return Vec::new();
        }
        *allowed = Some(allow);
        match key.clone() {
            PermissionKey::Claude { request_id, input } => {
                let response = if allow {
                    json!({"behavior": "allow", "updatedInput": input})
                } else {
                    json!({"behavior": "deny", "message": "The user declined this in Musializer."})
                };
                vec![json!({
                    "type": "control_response",
                    "response": {"subtype": "success", "request_id": request_id, "response": response}
                })
                .to_string()]
            }
            PermissionKey::Codex { rpc_id } => {
                let decision = if allow { "accept" } else { "decline" };
                vec![json!({"id": rpc_id, "result": {"decision": decision}}).to_string()]
            }
        }
    }

    /// Whether a permission prompt is waiting for an answer.
    #[must_use]
    pub fn awaiting_permission(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, Entry::Permission { allowed: None, .. }))
    }

    /// The process ended.
    pub fn exited(&mut self, how: &str) {
        self.busy = false;
        self.notice(format!("The agent stopped ({how})."), true);
    }

    /// One line the agent printed. Returns the lines to write back.
    pub fn receive(&mut self, line: &str) -> Vec<String> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            self.notice(truncate(line, 400), true);
            return Vec::new();
        };
        match self.protocol {
            Protocol::Claude => self.claude(&value),
            Protocol::Antigravity => {
                self.antigravity(&value);
                Vec::new()
            }
            Protocol::Codex => self.codex(&value),
        }
    }

    fn claude(&mut self, value: &Value) -> Vec<String> {
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "system" => {
                if value.get("subtype").and_then(Value::as_str) == Some("init") {
                    self.model = value
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.session = value
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
            }
            "stream_event" => {
                let event = &value["event"];
                match event.get("type").and_then(Value::as_str).unwrap_or("") {
                    "message_start" => {
                        self.message = event["message"]["id"].as_str().map(str::to_owned);
                    }
                    "content_block_delta" => {
                        if let Some(delta) = event["delta"]["text"].as_str() {
                            let key = format!(
                                "{}#{}",
                                self.message.as_deref().unwrap_or(""),
                                event["index"].as_u64().unwrap_or(0)
                            );
                            self.append_text(&key, delta);
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                let message = &value["message"];
                let id = message["id"].as_str().unwrap_or("").to_string();
                if let Some(blocks) = message["content"].as_array() {
                    for (index, block) in blocks.iter().enumerate() {
                        match block.get("type").and_then(Value::as_str).unwrap_or("") {
                            "text" => {
                                let text = block["text"].as_str().unwrap_or("");
                                // A message split across several `assistant`
                                // lines repeats its id with one block each, so
                                // the block's own index is the stream index
                                // only when it is the sole block.
                                let stream_index = if blocks.len() == 1 {
                                    self.open_text
                                        .keys()
                                        .filter(|key| key.starts_with(&format!("{id}#")))
                                        .filter_map(|key| {
                                            key.rsplit('#').next()?.parse::<usize>().ok()
                                        })
                                        .max()
                                        .unwrap_or(index)
                                } else {
                                    index
                                };
                                self.set_text(&format!("{id}#{stream_index}"), text);
                            }
                            "tool_use" => {
                                let tool_id = block["id"].as_str().unwrap_or("").to_string();
                                let name = block["name"].as_str().unwrap_or("tool").to_string();
                                self.start_tool(&tool_id, &name, tool_detail(&block["input"]));
                            }
                            _ => {}
                        }
                    }
                }
            }
            "user" => {
                if let Some(blocks) = value["message"]["content"].as_array() {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                            let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
                            let failed = block["is_error"].as_bool().unwrap_or(false);
                            self.finish_tool(&id, failed, &content_text(&block["content"]));
                        }
                    }
                }
            }
            "result" => {
                self.busy = false;
                self.open_text.clear();
                let cost = value["total_cost_usd"].as_f64();
                let seconds = value["duration_ms"].as_f64().map(|ms| ms / 1000.0);
                self.usage = match (cost, seconds) {
                    (Some(cost), Some(seconds)) => Some(format!("${cost:.3} · {seconds:.1} s")),
                    (Some(cost), None) => Some(format!("${cost:.3}")),
                    _ => None,
                };
                if value["is_error"].as_bool().unwrap_or(false) {
                    let text = value["result"].as_str().unwrap_or("the turn failed");
                    self.notice(truncate(text, 600), true);
                }
            }
            "control_response" => return self.claude_response(&value["response"]),
            "control_request" => {
                let request = &value["request"];
                if request.get("subtype").and_then(Value::as_str) == Some("can_use_tool") {
                    let input = request["input"].clone();
                    self.entries.push(Entry::Permission {
                        key: PermissionKey::Claude {
                            request_id: value["request_id"].as_str().unwrap_or("").to_string(),
                            input: input.clone(),
                        },
                        tool: request["tool_name"].as_str().unwrap_or("tool").to_string(),
                        detail: tool_detail(&input),
                        allowed: None,
                    });
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn antigravity(&mut self, value: &Value) {
        match value.get("event").and_then(Value::as_str).unwrap_or("") {
            "init" => {
                self.model = value["init"]["model"].as_str().map(str::to_owned);
                self.session = value["conversation_id"].as_str().map(str::to_owned);
                self.applied_effort = self.model.as_deref().and_then(|model| {
                    EFFORT_LEVELS
                        .iter()
                        .find(|level| model.ends_with(&format!("-{level}")))
                        .map(|level| (*level).to_owned())
                });
            }
            "step_update" => {
                let step = &value["step_update"];
                let key = format!("step-{}", step["step_index"].as_u64().unwrap_or(0));
                let done = step["state"].as_str() == Some("DONE");
                match step["step_type"].as_str().unwrap_or("") {
                    "agent_response" => {
                        if let Some(delta) = step["text_delta"].as_str() {
                            self.append_text(&key, delta);
                        }
                    }
                    "tool" => {
                        let name = step["tool_name"].as_str().unwrap_or("tool");
                        let detail = tool_detail(&step["tool_info"]["parameters"]);
                        self.start_tool(&key, name, detail);
                        if done {
                            let failed = step["error"].is_string();
                            self.finish_tool(&key, failed, step["error"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                let result = &value["result"];
                self.busy = false;
                self.open_text.clear();
                if let Some(id) = result["conversation_id"].as_str() {
                    self.session = Some(id.to_string());
                }
                if let Some(tokens) = result["usage"]["total_tokens"].as_u64() {
                    let seconds = result["duration_seconds"].as_f64().unwrap_or(0.0);
                    self.usage = Some(format!("{tokens} tokens · {seconds:.1} s"));
                }
                if result["status"].as_str() == Some("ERROR") {
                    let error = result["error"].as_str().unwrap_or("the turn failed");
                    self.notice(truncate(error, 600), true);
                }
                if let Some(denied) = result["denied_actions"].as_array() {
                    if !denied.is_empty() {
                        let names: Vec<&str> = denied
                            .iter()
                            .filter_map(|action| {
                                action["display_name"]
                                    .as_str()
                                    .or(action["action"].as_str())
                            })
                            .collect();
                        self.notice(
                            format!(
                                "Antigravity was not allowed to: {}. It cannot ask in this \
                                 mode; set its access to Full access to allow them.",
                                names.join(", ")
                            ),
                            true,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn codex(&mut self, value: &Value) -> Vec<String> {
        let mut out = Vec::new();
        let method = value.get("method").and_then(Value::as_str);
        let id = value.get("id");
        match (method, id) {
            // A response to one of ours.
            (None, Some(id)) => {
                let call = id
                    .as_u64()
                    .and_then(|id| self.codex.pending.remove(&id))
                    .unwrap_or(CodexCall::Other);
                if let Some(error) = value.get("error") {
                    let message = error["message"].as_str().unwrap_or("request failed");
                    if call == CodexCall::ModelList {
                        self.models_error = Some(truncate(message, 200));
                        return out;
                    }
                    self.notice(format!("Codex: {}", truncate(message, 400)), true);
                    if call != CodexCall::Other {
                        self.busy = false;
                    }
                    return out;
                }
                let result = &value["result"];
                match call {
                    CodexCall::Initialize => {
                        out.push(json!({"method": "initialized"}).to_string());
                        out.push(self.codex_request("model/list", json!({}), CodexCall::ModelList));
                        let access = self.selection.access;
                        let mut params = json!({
                            "cwd": self.codex.cwd,
                            "approvalPolicy": access.codex_approval(),
                            "approvalsReviewer": access.codex_reviewer(),
                            "sandbox": access.codex_sandbox_mode(),
                        });
                        if let Some(model) = &self.selection.model {
                            params["model"] = json!(model);
                        }
                        if !self.codex.instructions.is_empty() {
                            params["developerInstructions"] = json!(self.codex.instructions);
                        }
                        out.push(self.codex_request(
                            "thread/start",
                            params,
                            CodexCall::ThreadStart,
                        ));
                    }
                    CodexCall::ModelList => {
                        self.models.extend(
                            result["data"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|model| !model["hidden"].as_bool().unwrap_or(false))
                                .map(codex_model),
                        );
                        match result["nextCursor"].as_str() {
                            Some(cursor) => {
                                let cursor = cursor.to_owned();
                                out.push(self.codex_request(
                                    "model/list",
                                    json!({"cursor": cursor}),
                                    CodexCall::ModelList,
                                ));
                            }
                            None => out.extend(self.reconcile_selection()),
                        }
                    }
                    CodexCall::ThreadStart => {
                        self.codex.thread = result["thread"]["id"].as_str().map(str::to_owned);
                        self.applied_effort = result["reasoningEffort"].as_str().map(str::to_owned);
                        self.session.clone_from(&self.codex.thread);
                        self.model = result["model"]
                            .as_str()
                            .map(str::to_owned)
                            .or(self.model.take());
                        self.busy = !self.codex.queued.is_empty();
                        for text in std::mem::take(&mut self.codex.queued) {
                            out.push(self.codex_turn(&text));
                        }
                    }
                    CodexCall::TurnStart => {
                        self.codex.turn = result["turn"]["id"].as_str().map(str::to_owned);
                    }
                    CodexCall::Other => {}
                }
            }
            // A request from the server: approvals, and things this client
            // does not do, which get an error so the agent is not left waiting.
            (Some(method), Some(rpc_id)) => {
                let params = &value["params"];
                match method {
                    "item/commandExecution/requestApproval" | "execCommandApproval" => {
                        let command = params["command"]
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| {
                                params["command"].as_array().map(|parts| {
                                    parts
                                        .iter()
                                        .filter_map(Value::as_str)
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                })
                            })
                            .unwrap_or_default();
                        self.entries.push(Entry::Permission {
                            key: PermissionKey::Codex {
                                rpc_id: rpc_id.clone(),
                            },
                            tool: "command".to_string(),
                            detail: truncate(&command, 200),
                            allowed: None,
                        });
                    }
                    "item/fileChange/requestApproval" | "applyPatchApproval" => {
                        let reason = params["reason"].as_str().unwrap_or("edit files");
                        self.entries.push(Entry::Permission {
                            key: PermissionKey::Codex {
                                rpc_id: rpc_id.clone(),
                            },
                            tool: "file change".to_string(),
                            detail: truncate(reason, 200),
                            allowed: None,
                        });
                    }
                    _ => out.push(
                        json!({
                            "id": rpc_id,
                            "error": {"code": -32601, "message": format!("{method} is not supported by Musializer")}
                        })
                        .to_string(),
                    ),
                }
            }
            (Some(method), None) => self.codex_notification(method, &value["params"]),
            (None, None) => {}
        }
        out
    }

    fn codex_notification(&mut self, method: &str, params: &Value) {
        match method {
            "item/started" | "item/completed" => {
                let item = &params["item"];
                let id = item["id"].as_str().unwrap_or("").to_string();
                let completed = method == "item/completed";
                match item["type"].as_str().unwrap_or("") {
                    "agentMessage" => {
                        if completed {
                            self.set_text(&id, item["text"].as_str().unwrap_or(""));
                        }
                    }
                    "commandExecution" => {
                        let detail = item["commandActions"][0]["command"]
                            .as_str()
                            .or(item["command"].as_str())
                            .unwrap_or("")
                            .to_string();
                        self.start_tool(&id, "command", truncate(&detail, 160));
                        if completed {
                            let failed = item["exitCode"].as_i64().is_some_and(|code| code != 0)
                                || item["status"].as_str() == Some("failed");
                            let output = item["aggregatedOutput"].as_str().unwrap_or("");
                            self.finish_tool(&id, failed, output);
                        }
                    }
                    "fileChange" => {
                        let paths: Vec<&str> = item["changes"]
                            .as_array()
                            .map(|changes| {
                                changes
                                    .iter()
                                    .filter_map(|change| change["path"].as_str())
                                    .collect()
                            })
                            .unwrap_or_default();
                        self.start_tool(&id, "edit", truncate(&paths.join(", "), 160));
                        if completed {
                            let failed = item["status"].as_str() == Some("failed");
                            self.finish_tool(&id, failed, "");
                        }
                    }
                    "mcpToolCall" | "webSearch" | "dynamicToolCall" => {
                        let name = item["tool"]
                            .as_str()
                            .or(item["query"].as_str())
                            .unwrap_or("tool")
                            .to_string();
                        self.start_tool(&id, &name, tool_detail(&item["arguments"]));
                        if completed {
                            let failed = item["status"].as_str() == Some("failed");
                            self.finish_tool(&id, failed, "");
                        }
                    }
                    _ => {}
                }
            }
            "item/agentMessage/delta" => {
                let id = params["itemId"].as_str().unwrap_or("").to_string();
                self.append_text(&id, params["delta"].as_str().unwrap_or(""));
            }
            "turn/started" => {
                self.codex.turn = params["turn"]["id"].as_str().map(str::to_owned);
                self.busy = true;
            }
            "turn/completed" => {
                self.busy = false;
                self.codex.turn = None;
                self.open_text.clear();
                if let Some(error) = params["turn"]["error"]["message"].as_str() {
                    self.notice(truncate(error, 600), true);
                }
            }
            "thread/status/changed" => {
                if params["status"]["type"].as_str() == Some("idle") {
                    self.busy = false;
                }
            }
            "thread/tokenUsage/updated" => {
                if let Some(total) = params["tokenUsage"]["total"]["totalTokens"].as_u64() {
                    self.usage = Some(format!("{total} tokens"));
                }
            }
            "error" => {
                let message = params["error"]["message"]
                    .as_str()
                    .or(params["message"].as_str())
                    .unwrap_or("error");
                self.notice(format!("Codex: {}", truncate(message, 400)), true);
            }
            _ => {}
        }
    }

    /// The last thing the agent said, for the report line.
    #[must_use]
    pub fn last_reply(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Assistant { text } => Some(text.as_str()),
            _ => None,
        })
    }

    /// Counts for the report line: user turns, replies, tool calls, prompts.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for entry in &self.entries {
            match entry {
                Entry::User { .. } => counts.0 += 1,
                Entry::Assistant { .. } => counts.1 += 1,
                Entry::Tool { .. } => counts.2 += 1,
                Entry::Permission { .. } => counts.3 += 1,
                Entry::Notice { .. } | Entry::Image { .. } => {}
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(transcript: &mut Transcript, lines: &[Value]) -> Vec<String> {
        lines
            .iter()
            .flat_map(|line| transcript.receive(&line.to_string()))
            .collect()
    }

    #[test]
    fn claude_streams_text_runs_a_tool_and_does_not_repeat_the_final_message() {
        let mut t = Transcript::new(Protocol::Claude);
        let sent = t.send("make the bridge colder", None);
        assert_eq!(sent.len(), 1);
        let wire: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(wire["type"], "user");
        assert_eq!(
            wire["message"]["content"][0]["text"],
            "make the bridge colder"
        );
        assert!(t.busy);
        feed(
            &mut t,
            &[
                json!({"type":"system","subtype":"init","model":"claude-x","session_id":"s1"}),
                json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}),
                json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"Bash","input":{}}}}),
                json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"tools/headless_run.sh --still 3 a.png","description":"x"}}]}}),
                json!({"type":"user","message":{"content":[{"tool_use_id":"t1","type":"tool_result","content":"wrote a.png","is_error":false}]}}),
                json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m2"}}}),
                json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Done, "}}}),
                json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"the bridge is colder."}}}),
                json!({"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Done, the bridge is colder."}]}}),
                json!({"type":"result","subtype":"success","is_error":false,"duration_ms":4041,"total_cost_usd":0.0097,"result":"Done"}),
            ],
        );
        assert_eq!(t.model.as_deref(), Some("claude-x"));
        assert_eq!(t.session.as_deref(), Some("s1"));
        assert!(!t.busy);
        assert_eq!(t.usage.as_deref(), Some("$0.010 · 4.0 s"));
        let replies: Vec<&Entry> = t
            .entries
            .iter()
            .filter(|e| matches!(e, Entry::Assistant { .. }))
            .collect();
        assert_eq!(
            replies.len(),
            1,
            "the final message repeated the stream: {:?}",
            t.entries
        );
        assert_eq!(t.last_reply(), Some("Done, the bridge is colder."));
        assert!(t.entries.iter().any(|e| matches!(
            e,
            Entry::Tool { name, detail, state: ToolState::Done, output, .. }
                if name == "Bash" && detail.starts_with("tools/headless_run.sh") && output == "wrote a.png"
        )));
    }

    #[test]
    fn claude_permission_prompts_are_answered_on_the_wire() {
        let mut t = Transcript::new(Protocol::Claude);
        t.send("go", None);
        feed(
            &mut t,
            &[
                json!({"type":"control_request","request_id":"r7","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build"}}}),
            ],
        );
        assert!(t.awaiting_permission());
        let index = t.entries.len() - 1;
        let reply = t.decide(index, false);
        let wire: Value = serde_json::from_str(&reply[0]).unwrap();
        assert_eq!(wire["type"], "control_response");
        assert_eq!(wire["response"]["request_id"], "r7");
        assert_eq!(wire["response"]["response"]["behavior"], "deny");
        assert!(!t.awaiting_permission());
        // Answered once only.
        assert!(t.decide(index, true).is_empty());

        feed(
            &mut t,
            &[
                json!({"type":"control_request","request_id":"r8","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"a.json"}}}),
            ],
        );
        let reply = t.decide(t.entries.len() - 1, true);
        let wire: Value = serde_json::from_str(&reply[0]).unwrap();
        assert_eq!(wire["response"]["response"]["behavior"], "allow");
        assert_eq!(
            wire["response"]["response"]["updatedInput"]["file_path"],
            "a.json"
        );
    }

    #[test]
    fn antigravity_steps_become_text_and_tools_and_denials_are_explained() {
        let mut t = Transcript::new(Protocol::Antigravity);
        let sent = t.send("look", Some("context"));
        let wire: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(wire["event"], "user");
        assert_eq!(wire["message"]["content"], "context\n\nlook");
        feed(
            &mut t,
            &[
                json!({"event":"init","conversation_id":"c1","init":{"model":"gemini-x"}}),
                json!({"event":"step_update","step_update":{"step_index":0,"state":"DONE","step_type":"user_input"}}),
                json!({"event":"step_update","step_update":{"step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"po"}}),
                json!({"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"ng"}}),
                json!({"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hi"}}}}),
                json!({"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"echo hi"}}}}),
                json!({"event":"result","result":{"conversation_id":"c1","status":"SUCCESS","duration_seconds":2.2,"usage":{"total_tokens":11872},"denied_actions":[{"action":"command","display_name":"RunCommand"}]}}),
            ],
        );
        assert_eq!(t.model.as_deref(), Some("gemini-x"));
        assert_eq!(t.last_reply(), Some("pong"));
        assert!(!t.busy);
        assert_eq!(t.usage.as_deref(), Some("11872 tokens · 2.2 s"));
        assert!(t.entries.iter().any(|e| matches!(e, Entry::Tool { detail, state: ToolState::Done, .. } if detail == "echo hi")));
        assert!(t.entries.iter().any(
            |e| matches!(e, Entry::Notice { text, error: true } if text.contains("RunCommand"))
        ));
    }

    #[test]
    fn codex_handshakes_before_the_first_turn_and_answers_approvals() {
        let mut t = Transcript::new(Protocol::Codex);
        let start = t.start("/music", "direct the video");
        let init: Value = serde_json::from_str(&start[0]).unwrap();
        assert_eq!(init["method"], "initialize");
        // A message typed before the thread exists waits for it.
        assert!(t.send("hello", None).is_empty());
        let after_init = t.receive(&json!({"id": init["id"], "result": {}}).to_string());
        let methods: Vec<String> = after_init
            .iter()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["method"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(methods, ["initialized", "model/list", "thread/start"]);
        let thread_start: Value = serde_json::from_str(&after_init[2]).unwrap();
        assert_eq!(thread_start["params"]["cwd"], "/music");
        assert_eq!(
            thread_start["params"]["developerInstructions"],
            "direct the video"
        );
        let after_thread = t.receive(
            &json!({"id": thread_start["id"], "result": {"thread": {"id": "th1"}, "model": "gpt-x"}}).to_string(),
        );
        let turn: Value = serde_json::from_str(&after_thread[0]).unwrap();
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(turn["params"]["threadId"], "th1");
        assert_eq!(turn["params"]["input"][0]["text"], "hello");
        assert_eq!(t.model.as_deref(), Some("gpt-x"));

        let replies = feed(
            &mut t,
            &[
                json!({"method":"turn/started","params":{"turn":{"id":"tu1"}}}),
                json!({"method":"item/started","params":{"item":{"type":"agentMessage","id":"a1","text":""}}}),
                json!({"method":"item/agentMessage/delta","params":{"itemId":"a1","delta":"Running"}}),
                json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"a1","text":"Running it."}}}),
                json!({"method":"item/started","params":{"item":{"type":"commandExecution","id":"x1","command":"/bin/zsh -lc 'ls'","commandActions":[{"command":"ls"}],"status":"inProgress"}}}),
                json!({"id": 41, "method":"item/commandExecution/requestApproval","params":{"itemId":"x1","command":"ls"}}),
                json!({"id": 42, "method":"item/tool/requestUserInput","params":{}}),
                json!({"method":"item/completed","params":{"item":{"type":"commandExecution","id":"x1","commandActions":[{"command":"ls"}],"status":"completed","exitCode":0,"aggregatedOutput":"a\nb\n"}}}),
                json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"total":{"totalTokens":37818}}}}),
                json!({"method":"turn/completed","params":{"turn":{"id":"tu1"}}}),
            ],
        );
        // The unsupported request got an error rather than silence.
        let refusal: Value = serde_json::from_str(&replies[0]).unwrap();
        assert_eq!(refusal["id"], 42);
        assert!(refusal["error"].is_object());
        assert_eq!(t.last_reply(), Some("Running it."));
        assert!(!t.busy);
        assert_eq!(t.usage.as_deref(), Some("37818 tokens"));
        let permission = t
            .entries
            .iter()
            .position(|e| matches!(e, Entry::Permission { .. }))
            .unwrap();
        let answer: Value = serde_json::from_str(&t.decide(permission, true)[0]).unwrap();
        assert_eq!(answer, json!({"id": 41, "result": {"decision": "accept"}}));
        assert!(t.entries.iter().any(|e| matches!(e, Entry::Tool { detail, output, state: ToolState::Done, .. } if detail == "ls" && output == "a\nb\n")));
    }

    fn wire(line: &str) -> Value {
        serde_json::from_str(line).unwrap()
    }

    fn claude_listing() -> Value {
        json!({"models": [
            {"value": "default", "resolvedModel": "claude-opus-5-5", "displayName": "Default (recommended)",
             "description": "Opus 5.5", "supportedEffortLevels": ["low", "high", "max"], "supportsAutoMode": true},
            {"value": "opus", "resolvedModel": "claude-opus-5-5", "displayName": "Opus 5.5",
             "supportedEffortLevels": ["low", "high", "max"], "supportsAutoMode": true},
            {"value": "claude-haiku-4-5", "resolvedModel": "claude-haiku-4-5", "displayName": "Haiku 4.5"},
        ], "current_permission_mode": "acceptEdits"})
    }

    /// Answers `line`, a request of ours, as Claude would.
    fn claude_answers(t: &mut Transcript, line: &str, body: Value) -> Vec<String> {
        let id = wire(line)["request_id"].clone();
        t.receive(
            &json!({"type": "control_response", "response": {"subtype": "success", "request_id": id, "response": body}})
                .to_string(),
        )
    }

    #[test]
    fn claude_lists_its_models_and_says_what_it_applied_before_any_turn() {
        let mut t = Transcript::new(Protocol::Claude);
        let start = t.start("/music", "");
        assert_eq!(wire(&start[0])["request"]["subtype"], "initialize");
        assert_eq!(wire(&start[1])["request"]["subtype"], "get_settings");
        assert_eq!(t.model_label(), "Default");
        assert_eq!(t.effort_label(), "Default");
        assert!(claude_answers(&mut t, &start[0], claude_listing()).is_empty());
        claude_answers(
            &mut t,
            &start[1],
            json!({"applied": {"model": "claude-opus-5-5", "effort": "high"}}),
        );
        assert_eq!(t.models.len(), 3);
        // The alias never names the chip while the model it stands for has one.
        assert_eq!(t.model_label(), "Opus 5.5");
        assert_eq!(t.effort_label(), "High");
        assert_eq!(t.reported_mode.as_deref(), Some("acceptEdits"));
        assert!(t.entries.is_empty(), "the catalogue is not a conversation");
        assert!(!t.busy);
    }

    #[test]
    fn claude_takes_each_choice_live_and_reads_back_what_applied() {
        let mut t = Transcript::new(Protocol::Claude);
        let start = t.start("/music", "");
        claude_answers(&mut t, &start[0], claude_listing());

        let model = t.choose_model("opus");
        assert_eq!(wire(&model[0])["request"]["subtype"], "set_model");
        assert_eq!(wire(&model[0])["request"]["model"], "opus");
        let settings = claude_answers(&mut t, &model[0], json!({"model": "claude-opus-5-5"}));
        assert_eq!(wire(&settings[0])["request"]["subtype"], "get_settings");

        claude_answers(
            &mut t,
            &settings[0],
            json!({"applied": {"model": "claude-opus-5-5", "effort": "high"}}),
        );
        let effort = t.choose_effort("max");
        assert_eq!(
            wire(&effort[0])["request"]["subtype"],
            "apply_flag_settings"
        );
        assert_eq!(
            wire(&effort[0])["request"]["settings"]["effortLevel"],
            "max"
        );
        let settings = claude_answers(&mut t, &effort[0], json!({}));
        assert_eq!(t.effort_label(), "High", "not read back yet");
        claude_answers(
            &mut t,
            &settings[0],
            json!({"applied": {"model": "claude-opus-5-5", "effort": "max"}}),
        );
        assert_eq!(t.effort_label(), "Max");

        let access = t.choose_access(Access::Full);
        assert_eq!(
            wire(&access[0])["request"]["subtype"],
            "set_permission_mode"
        );
        assert_eq!(wire(&access[0])["request"]["mode"], "bypassPermissions");
        claude_answers(&mut t, &access[0], json!({"mode": "bypassPermissions"}));
        assert_eq!(t.reported_mode.as_deref(), Some("bypassPermissions"));
        assert!(!t.take_restart_owed());
    }

    #[test]
    fn a_refused_choice_goes_back_and_says_why() {
        let mut t = Transcript::new(Protocol::Claude);
        let lines = t.choose_access(Access::Full);
        assert_eq!(t.selection.access, Access::Full);
        let id = wire(&lines[0])["request_id"].clone();
        t.receive(
            &json!({"type": "control_response", "response": {"subtype": "error", "request_id": id, "error": "bypass_not_launched"}})
                .to_string(),
        );
        assert_eq!(t.selection.access, Access::AcceptEdits);
        assert!(t.take_selection_reverted());
        assert!(t.entries.iter().any(
            |e| matches!(e, Entry::Notice { text, error: true } if text.contains("bypass_not_launched"))
        ));
    }

    #[test]
    fn a_new_model_drops_what_it_cannot_do() {
        let mut t = Transcript::new(Protocol::Claude);
        let start = t.start("/music", "");
        claude_answers(&mut t, &start[0], claude_listing());
        t.selection.effort = Some("max".into());
        t.selection.access = Access::Auto;
        let lines = t.choose_model("claude-haiku-4-5");
        assert_eq!(t.selection.effort, None, "this model has no levels");
        assert_eq!(t.selection.access, Access::AcceptEdits);
        let subtypes: Vec<Value> = lines
            .iter()
            .map(|line| wire(line)["request"]["subtype"].clone())
            .collect();
        assert_eq!(subtypes, ["set_model", "set_permission_mode"]);
        assert!(t
            .access_refusal(Access::Auto)
            .unwrap()
            .contains("Haiku 4.5"));
        assert_eq!(t.access_refusal(Access::Full), None);
    }

    #[test]
    fn a_remembered_model_the_cli_dropped_does_not_fail_the_first_turn() {
        let mut t = Transcript::new(Protocol::Claude);
        t.selection.model = Some("claude-gone".into());
        t.selection.effort = Some("high".into());
        let start = t.start("/music", "");
        let reply = claude_answers(&mut t, &start[0], claude_listing());
        assert_eq!(wire(&reply[0])["request"]["model"], "default");
        assert_eq!(t.selection.model, None);
        assert_eq!(t.selection.effort, None);
        assert!(t.take_selection_reverted());
    }

    #[test]
    fn launch_flags_carry_the_selection_into_a_new_process() {
        let mut t = Transcript::new(Protocol::Claude);
        t.selection = Selection {
            model: Some("opus".into()),
            effort: Some("xhigh".into()),
            access: Access::Supervised,
        };
        assert_eq!(
            t.launch_flags(),
            [
                "--permission-mode",
                "default",
                "--allow-dangerously-skip-permissions",
                "--model",
                "opus",
                "--effort",
                "xhigh"
            ]
        );
        let mut t = Transcript::new(Protocol::Antigravity);
        assert_eq!(t.launch_flags(), ["--mode", "accept-edits"]);
        t.selection = Selection {
            model: Some("gemini-x".into()),
            effort: Some("high".into()),
            access: Access::Full,
        };
        assert_eq!(
            t.launch_flags(),
            [
                "--mode",
                "accept-edits",
                "--dangerously-skip-permissions",
                "--model",
                "gemini-x-high"
            ]
        );
        t.selection.access = Access::Auto;
        assert_eq!(t.launch_flags()[..2], ["--mode", "auto"]);
        assert!(Transcript::new(Protocol::Codex).launch_flags().is_empty());
    }

    #[test]
    fn every_access_level_has_its_wire_words_on_each_protocol() {
        #[rustfmt::skip]
        let table = [
            (Access::Supervised,  "default",           "untrusted",  "user",        "read-only",          "readOnly"),
            (Access::AcceptEdits, "acceptEdits",       "on-request", "user",        "workspace-write",    "workspaceWrite"),
            (Access::Auto,        "auto",              "on-request", "auto_review", "workspace-write",    "workspaceWrite"),
            (Access::Full,        "bypassPermissions", "never",      "user",        "danger-full-access", "dangerFullAccess"),
        ];
        for (access, claude, approval, reviewer, sandbox, policy) in table {
            assert_eq!(access.claude_mode(), claude);
            assert_eq!(access.codex_approval(), approval);
            assert_eq!(access.codex_reviewer(), reviewer);
            assert_eq!(access.codex_sandbox_mode(), sandbox);
            assert_eq!(access.codex_sandbox_policy()["type"], policy);
            assert_eq!(Access::from_key(access.key()), Some(access));
        }
        assert_eq!(Access::from_key("everything"), None);
    }

    #[test]
    fn codex_lists_models_and_each_turn_carries_the_selection() {
        let mut t = Transcript::new(Protocol::Codex);
        t.selection.access = Access::Full;
        let start = t.start("/music", "");
        let after_init = t.receive(&json!({"id": wire(&start[0])["id"], "result": {}}).to_string());
        let methods: Vec<Value> = after_init
            .iter()
            .map(|l| wire(l)["method"].clone())
            .collect();
        assert_eq!(methods, ["initialized", "model/list", "thread/start"]);
        let thread_start = wire(&after_init[2]);
        assert_eq!(thread_start["params"]["approvalPolicy"], "never");
        assert_eq!(thread_start["params"]["sandbox"], "danger-full-access");
        assert_eq!(thread_start["params"]["approvalsReviewer"], "user");

        let page = t.receive(
            &json!({"id": wire(&after_init[1])["id"], "result": {"nextCursor": "c2", "data": [
                {"id": "gpt-a", "model": "gpt-a", "displayName": "GPT A", "hidden": false,
                 "defaultReasoningEffort": "low",
                 "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "ultra"}]},
                {"id": "gpt-hidden", "model": "gpt-hidden", "displayName": "Hidden", "hidden": true},
            ]}})
            .to_string(),
        );
        assert_eq!(wire(&page[0])["params"]["cursor"], "c2");
        t.receive(
            &json!({"id": wire(&page[0])["id"], "result": {"data": [
                {"id": "gpt-b", "model": "gpt-b", "displayName": "GPT B", "hidden": false,
                 "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [{"reasoningEffort": "medium"}]},
            ]}})
            .to_string(),
        );
        assert_eq!(
            t.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-a", "gpt-b"]
        );
        t.receive(
            &json!({"id": thread_start["id"], "result": {"thread": {"id": "th"}, "model": "gpt-b", "reasoningEffort": "medium"}})
                .to_string(),
        );
        assert_eq!(t.model_label(), "GPT B");
        assert_eq!(t.effort_label(), "Medium");

        // Nothing is written until the turn that carries it.
        assert!(t.choose_model("gpt-a").is_empty());
        assert_eq!(t.effort_label(), "Low", "the new model's own default");
        assert!(t.choose_effort("ultra").is_empty());
        assert!(t.choose_access(Access::Auto).is_empty());
        let turn = wire(&t.send("go", None)[0]);
        assert_eq!(turn["params"]["model"], "gpt-a");
        assert_eq!(turn["params"]["effort"], "ultra");
        assert_eq!(turn["params"]["approvalPolicy"], "on-request");
        assert_eq!(turn["params"]["approvalsReviewer"], "auto_review");
        assert_eq!(turn["params"]["sandboxPolicy"]["type"], "workspaceWrite");
    }

    #[test]
    fn codex_without_a_model_list_keeps_its_turn_alive() {
        let mut t = Transcript::new(Protocol::Codex);
        let start = t.start("/music", "");
        let after_init = t.receive(&json!({"id": wire(&start[0])["id"], "result": {}}).to_string());
        t.receive(
            &json!({"id": wire(&after_init[1])["id"], "error": {"message": "no catalogue"}})
                .to_string(),
        );
        assert_eq!(t.models_error.as_deref(), Some("no catalogue"));
        assert!(t.busy, "the handshake is still running");
        assert!(t.entries.is_empty());
    }

    #[test]
    fn agy_models_fold_to_one_entry_per_model() {
        let models = antigravity_models(
            "Fetching available models...\n\
             gemini-4-argon-medium\tGemini 4 Argon (Medium)\n\
             gemini-4-argon-high\tGemini 4 Argon (High)\n\
             gemini-4-argon-low\tGemini 4 Argon (Low)\n\
             gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n\
             plain-model\tPlain Model\n",
        );
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "gemini-4-argon");
        assert_eq!(models[0].label, "Gemini 4 Argon");
        assert_eq!(models[0].efforts, ["low", "medium", "high"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("medium"));
        assert_eq!(models[1].efforts, ["medium"]);
        assert_eq!(models[2].id, "plain-model");
        assert!(models[2].efforts.is_empty());
        assert_eq!(models[2].default_effort, None);
    }

    #[test]
    fn antigravity_choices_wait_for_a_new_process() {
        let mut t = Transcript::new(Protocol::Antigravity);
        t.set_models(antigravity_models(
            "m-low\tM (Low)\nm-high\tM (High)\nn-medium\tN (Medium)\n",
        ));
        assert!(t.choose_access(Access::Full).is_empty());
        assert!(t.take_restart_owed());
        assert!(!t.take_restart_owed(), "asking clears it");
        assert!(t.choose_model("m").is_empty());
        assert!(t.take_restart_owed());
        assert_eq!(t.selection.effort.as_deref(), Some("low"));
        t.model = Some("m-high".into());
        t.selection.model = None;
        assert_eq!(t.model_label(), "M", "the reported id names its model");
        assert!(t.choose_effort("high").is_empty());
        assert_eq!(t.selection.model.as_deref(), Some("m"));
        assert!(t.launch_flags().contains(&"m-high".to_owned()));
        assert!(t.access_refusal(Access::Supervised).is_some());
        assert_eq!(t.access_refusal(Access::Auto), None);
    }

    #[test]
    fn antigravity_reports_its_level_in_the_model_id() {
        let mut t = Transcript::new(Protocol::Antigravity);
        t.set_models(antigravity_models("m-low\tM (Low)\nm-medium\tM (Medium)\n"));
        t.receive(
            &json!({"event": "init", "conversation_id": "c", "init": {"model": "m-low"}})
                .to_string(),
        );
        assert_eq!(t.model_label(), "M");
        assert_eq!(t.effort_label(), "Low");
    }

    #[test]
    fn antigravity_saying_it_has_no_auto_mode_puts_the_choice_back() {
        let mut t = Transcript::new(Protocol::Antigravity);
        t.choose_access(Access::Auto);
        t.diagnostic("warning: --mode auto is not available for this account; continuing");
        assert_eq!(t.selection.access, Access::AcceptEdits);
        assert!(t.take_selection_reverted());
        assert!(t.access_refusal(Access::Auto).is_some());
        // One word about it, and only Antigravity's stderr means this.
        t.diagnostic("warning: --mode auto is not available for this account; continuing");
        assert_eq!(t.entries.len(), 1);
        let mut claude = Transcript::new(Protocol::Claude);
        claude.selection.access = Access::Auto;
        claude.diagnostic("warning: --mode auto is not available");
        assert_eq!(claude.selection.access, Access::Auto);
    }

    #[test]
    fn effort_titles() {
        assert_eq!(effort_title("xhigh"), "XHigh");
        assert_eq!(effort_title("medium"), "Medium");
        assert_eq!(effort_title(""), "");
    }

    #[test]
    fn prose_on_stdout_is_shown_rather_than_dropped() {
        let mut t = Transcript::new(Protocol::Claude);
        t.receive("Error: not logged in");
        assert!(
            matches!(&t.entries[0], Entry::Notice { text, error: true } if text.contains("not logged in"))
        );
        // Unknown but well-formed events are ignored.
        t.receive(&json!({"type":"rate_limit_event","rate_limit_info":{}}).to_string());
        assert_eq!(t.entries.len(), 1);
    }
}
