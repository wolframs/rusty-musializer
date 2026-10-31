//! What each agent's composer last chose (model, effort, access).
//!
//! A third per-user store beside `ui.json` and `recent.json`, for the reasons
//! [`super::preferences::recent`] gives: `UiPreferences` is `Copy`, and one
//! truncated file must not cost the others. The same contract too: a versioned
//! schema checked exactly, a size bound, an atomic replace, and a load that
//! tells "no file yet" from "a file I refuse to touch".
//!
//! **Full access is not remembered.** It is chosen per chat, and a new chat
//! starts at the default access, so nothing here can start an agent without
//! prompts on a later launch (`docs/WORKSTATION.md`).

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use musializer_core::agent::{Access, Selection};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "musializer.agent-choices/v1";
const MAX_FILE_SIZE: u64 = 16 * 1024;
const MAX_AGENTS: usize = 16;
const MAX_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentChoices {
    agents: BTreeMap<String, Selection>,
}

impl AgentChoices {
    /// What a new chat with `agent` starts on.
    #[must_use]
    pub fn get(&self, agent: &str) -> Selection {
        self.agents.get(agent).cloned().unwrap_or_default()
    }

    pub fn remember(&mut self, agent: &str, selection: &Selection) {
        let mut kept = selection.clone();
        if kept.access == Access::Full {
            kept.access = Access::default();
        }
        self.agents.insert(agent.to_owned(), kept);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentChoicesError {
    #[error("could not read the agent choices")]
    Read,
    #[error("the agent choices file has an invalid format")]
    Format,
    #[error("could not write the agent choices")]
    Write,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    agents: BTreeMap<String, Entry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    model: Option<String>,
    effort: Option<String>,
    access: String,
}

fn sane_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_BYTES && !name.chars().any(char::is_control)
}

/// `$MUSIALIZER_AGENT_CHOICES`, else the XDG config directory.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSIALIZER_AGENT_CHOICES") {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer/agents.json"));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".config/musializer/agents.json"))
}

/// `Ok(None)` is "no file yet", which is a new user rather than a fault.
pub fn load(path: &Path) -> Result<Option<AgentChoices>, AgentChoicesError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(AgentChoicesError::Read),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AgentChoicesError::Read)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(AgentChoicesError::Format);
    }
    let document: Document =
        serde_json::from_slice(&bytes).map_err(|_| AgentChoicesError::Format)?;
    if document.schema != SCHEMA || document.agents.len() > MAX_AGENTS {
        return Err(AgentChoicesError::Format);
    }
    let mut choices = AgentChoices::default();
    for (agent, entry) in document.agents {
        let access = Access::from_key(&entry.access).ok_or(AgentChoicesError::Format)?;
        // This application never writes it, so a file that says it was edited
        // by hand, and is refused rather than obeyed.
        if access == Access::Full
            || !sane_name(&agent)
            || !entry
                .model
                .iter()
                .chain(&entry.effort)
                .all(|n| sane_name(n))
        {
            return Err(AgentChoicesError::Format);
        }
        choices.agents.insert(
            agent,
            Selection {
                model: entry.model,
                effort: entry.effort,
                access,
            },
        );
    }
    Ok(Some(choices))
}

pub fn save(path: &Path, choices: &AgentChoices) -> Result<(), AgentChoicesError> {
    let document = Document {
        schema: SCHEMA.to_string(),
        agents: choices
            .agents
            .iter()
            .map(|(agent, selection)| {
                (
                    agent.clone(),
                    Entry {
                        model: selection.model.clone(),
                        effort: selection.effort.clone(),
                        access: selection.access.key().to_string(),
                    },
                )
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|_| AgentChoicesError::Format)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| AgentChoicesError::Write)?;
    }
    musializer_runtime::process::publish::atomic_write(path, &bytes)
        .map_err(|_| AgentChoicesError::Write)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mz-agent-choices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn choices_round_trip_and_a_missing_file_is_a_new_user() {
        let path = file("round.json");
        let _ = std::fs::remove_file(&path);
        assert_eq!(load(&path).unwrap(), None);
        let mut choices = AgentChoices::default();
        let selection = Selection {
            model: Some("opus".into()),
            effort: Some("high".into()),
            access: Access::Supervised,
        };
        choices.remember("claude", &selection);
        save(&path, &choices).unwrap();
        let read = load(&path).unwrap().unwrap();
        assert_eq!(read.get("claude"), selection);
        assert_eq!(read.get("codex"), Selection::default());
    }

    #[test]
    fn full_access_is_never_remembered_and_never_obeyed_from_a_file() {
        let mut choices = AgentChoices::default();
        choices.remember(
            "claude",
            &Selection {
                access: Access::Full,
                ..Selection::default()
            },
        );
        assert_eq!(choices.get("claude").access, Access::default());
        let path = file("full.json");
        std::fs::write(
            &path,
            r#"{"schema":"musializer.agent-choices/v1","agents":{"claude":{"model":null,"effort":null,"access":"full"}}}"#,
        )
        .unwrap();
        assert!(matches!(load(&path), Err(AgentChoicesError::Format)));
    }

    #[test]
    fn a_foreign_file_is_refused_rather_than_read() {
        let path = file("foreign.json");
        std::fs::write(
            &path,
            r#"{"schema":"musializer.agent-choices/v2","agents":{}}"#,
        )
        .unwrap();
        assert!(matches!(load(&path), Err(AgentChoicesError::Format)));
        std::fs::write(&path, "{").unwrap();
        assert!(matches!(load(&path), Err(AgentChoicesError::Format)));
    }
}
