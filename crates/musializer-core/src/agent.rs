//! A conversation with a coding agent CLI, as the dock's chat tab shows it
//! (WX-4; design in `docs/WORKSTATION.md`).
//!
//! Three CLIs, three wire protocols, one transcript:
//!
//! | Agent | Process | Protocol |
//! | --- | --- | --- |
//! | Claude Code | `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio` | NDJSON `type`-tagged events; permission prompts arrive as `control_request` and are answered with `control_response` |
//! | Antigravity | `agy --input-format stream-json --output-format stream-json --print=` | NDJSON `event`-tagged `step_update`s; it cannot ask for a permission in this mode, so a denied action is reported in its `result` |
//! | Codex | `codex app-server` | JSON-RPC over stdio: `initialize`, `thread/start`, `turn/start`; approvals arrive as server requests |
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

use std::collections::HashMap;

use serde_json::{json, Value};

/// Which wire protocol a transcript speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Claude,
    Antigravity,
    Codex,
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
    ThreadStart,
    TurnStart,
    Other,
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
    /// Text being streamed, keyed by the protocol's own handle for it, so the
    /// final message replaces the streamed text rather than repeating it.
    open_text: HashMap<String, usize>,
    /// Tool entries by the protocol's id.
    tools: HashMap<String, usize>,
    /// Claude's id for the message being streamed.
    message: Option<String>,
    codex: CodexState,
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
            open_text: HashMap::new(),
            tools: HashMap::new(),
            message: None,
            codex: CodexState::default(),
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
                vec![self.codex_request(
                    "initialize",
                    json!({"clientInfo": {"name": "musializer", "title": "Musializer", "version": env!("CARGO_PKG_VERSION")}}),
                    CodexCall::Initialize,
                )]
            }
            Protocol::Claude | Protocol::Antigravity => Vec::new(),
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
        self.codex_request(
            "turn/start",
            json!({"threadId": thread, "input": [{"type": "text", "text": text}]}),
            CodexCall::TurnStart,
        )
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
                                 mode; turn on \"let it run commands\" to allow them.",
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
                        let mut params = json!({
                            "cwd": self.codex.cwd,
                            "approvalPolicy": "on-request",
                        });
                        if !self.codex.instructions.is_empty() {
                            params["developerInstructions"] = json!(self.codex.instructions);
                        }
                        out.push(self.codex_request(
                            "thread/start",
                            params,
                            CodexCall::ThreadStart,
                        ));
                    }
                    CodexCall::ThreadStart => {
                        self.codex.thread = result["thread"]["id"].as_str().map(str::to_owned);
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
                Entry::Notice { .. } => {}
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
        assert_eq!(methods, ["initialized", "thread/start"]);
        let thread_start: Value = serde_json::from_str(&after_init[1]).unwrap();
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
