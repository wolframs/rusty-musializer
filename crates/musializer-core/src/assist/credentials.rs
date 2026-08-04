//! `musializer.assist-credentials/v1`: the one file that holds a key.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §3. This module owns the *document*: its
//! schema, its key grammar, and the `Forget` semantics. The permissions — file
//! `0600`, directory `0700`, both set before any secret bytes exist, and a
//! loose-permission file refused rather than repaired — belong to
//! `musializer_runtime::assist::files`, because they are `open(2)` flags and
//! this crate opens nothing.
//!
//! Operator decision, 2026-08-04: no OS or vendor wallet. kwallet and the
//! Secret Service were rejected for cross-platform reasons, so this is the
//! `gh`/`aws` model — a mode-`0600` file under the per-user config directory,
//! with session-only storage and environment import as the alternatives.
//!
//! Serialization is deterministic (fixed field order, `BTreeMap` entries), which
//! is what makes `Forget` provable: removing one provider must leave every other
//! provider's bytes untouched, and a test compares the block rather than
//! trusting the writer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::assist::secret::Secret;

/// The schema token.
pub const SCHEMA: &str = "musializer.assist-credentials/v1";

/// Size cap, applied before parsing.
pub const MAX_FILE_SIZE: u64 = 64 * 1024;

/// How many provider entries the file may carry.
pub const MAX_ENTRIES: usize = 64;

/// The longest provider slug or lookup id.
pub const MAX_KEY_PART: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CredentialsError {
    #[error("the credentials file is larger than the {MAX_FILE_SIZE} byte cap")]
    TooLarge,
    #[error("the credentials file is not valid JSON for this schema")]
    Format,
    #[error("the credentials file declares schema {0:?}, not {SCHEMA}")]
    Schema(String),
    #[error("{0}")]
    Invalid(String),
}

/// One provider account.
///
/// `label` is the provider-returned display name from the last successful
/// `Test`. The fingerprint is derived from the secret rather than stored, so the
/// file cannot disagree with itself.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialEntry {
    pub secret: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl CredentialEntry {
    /// `sha256(secret)[0..8]` — the only identity of this key that may be
    /// displayed or written anywhere else.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.secret.fingerprint()
    }
}

/// The whole file.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialStore {
    pub schema: String,
    #[serde(default)]
    entries: BTreeMap<String, CredentialEntry>,
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self {
            schema: SCHEMA.to_string(),
            entries: BTreeMap::new(),
        }
    }
}

/// The map key: `provider/lookup_id` (§3).
///
/// Returns `None` rather than a mangled key when either part is unusable, so a
/// caller cannot construct an entry it will never be able to find again.
#[must_use]
pub fn entry_key(provider: &str, lookup_id: &str) -> Option<String> {
    if !is_key_part(provider) || !is_key_part(lookup_id) {
        return None;
    }
    Some(format!("{provider}/{lookup_id}"))
}

fn is_key_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEY_PART
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_' | '.')
        })
        && !value.contains("..")
        && !value.starts_with('.')
}

impl CredentialStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses and validates.
    pub fn parse(bytes: &[u8]) -> Result<Self, CredentialsError> {
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(CredentialsError::TooLarge);
        }
        let store: Self = serde_json::from_slice(bytes).map_err(|_| CredentialsError::Format)?;
        if store.schema != SCHEMA {
            return Err(CredentialsError::Schema(store.schema));
        }
        store.validate()?;
        Ok(store)
    }

    /// Serializes. Deterministic, which is what `Forget`'s byte-identity check
    /// rests on.
    pub fn to_bytes(&self) -> Result<Vec<u8>, CredentialsError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|_| CredentialsError::Format)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(CredentialsError::TooLarge);
        }
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), CredentialsError> {
        if self.schema != SCHEMA {
            return Err(CredentialsError::Schema(self.schema.clone()));
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(CredentialsError::Invalid(format!(
                "a credentials file may carry at most {MAX_ENTRIES} entries"
            )));
        }
        for (key, entry) in &self.entries {
            let Some((provider, lookup_id)) = key.split_once('/') else {
                return Err(CredentialsError::Invalid(
                    "an entry key must be provider/lookup_id".to_string(),
                ));
            };
            if entry_key(provider, lookup_id).as_deref() != Some(key.as_str()) {
                return Err(CredentialsError::Invalid(format!(
                    "entry key {key:?} is not a well-formed provider/lookup_id"
                )));
            }
            if entry.secret.is_empty() {
                return Err(CredentialsError::Invalid(format!(
                    "entry {key:?} holds an empty credential"
                )));
            }
            if let Some(label) = &entry.label {
                if label.is_empty()
                    || label.len() > MAX_KEY_PART * 4
                    || label.chars().any(char::is_control)
                {
                    return Err(CredentialsError::Invalid(format!(
                        "entry {key:?} has an unusable label"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Stores or replaces a provider's credential.
    ///
    /// Returns `false` for an unusable key rather than storing under a mangled
    /// one; `Replace` is the only flow that reaches this, and it has a dialog to
    /// report into.
    pub fn insert(
        &mut self,
        provider: &str,
        lookup_id: &str,
        secret: Secret,
        label: Option<String>,
    ) -> bool {
        let Some(key) = entry_key(provider, lookup_id) else {
            return false;
        };
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
            return false;
        }
        self.entries.insert(key, CredentialEntry { secret, label });
        true
    }

    #[must_use]
    pub fn get(&self, provider: &str, lookup_id: &str) -> Option<&CredentialEntry> {
        self.entries.get(&entry_key(provider, lookup_id)?)
    }

    /// `Forget`: removes this provider's entry and reports whether one was
    /// there. Other providers' entries are untouched — the caller rewrites the
    /// file from this store, and the deterministic serialization means their
    /// bytes are identical.
    pub fn forget(&mut self, provider: &str, lookup_id: &str) -> bool {
        entry_key(provider, lookup_id)
            .and_then(|key| self.entries.remove(&key))
            .is_some()
    }

    /// The stored `provider/lookup_id` keys, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "sk-or-v1-MUSICANARY7Q4X2ZK9aaaabbbbcccc";

    fn two_providers() -> CredentialStore {
        let mut store = CredentialStore::new();
        assert!(store.insert(
            "openrouter",
            "default",
            Secret::new(CANARY.to_string()),
            Some("Personal".to_string()),
        ));
        assert!(store.insert(
            "together",
            "work",
            Secret::new("sk-together-second-key".to_string()),
            Some("Work".to_string()),
        ));
        store
    }

    /// The JSON object text for one entry, so `Forget` can be checked as byte
    /// identity rather than as "it still parses".
    fn entry_block(bytes: &[u8], key: &str) -> String {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let opening = format!("\"{key}\": {{");
        let start = text.find(&opening).expect("entry is present");
        let end = text[start..].find("\n    }").expect("entry closes") + start + "\n    }".len();
        text[start..end].to_string()
    }

    #[test]
    fn a_store_round_trips() {
        let store = two_providers();
        let bytes = store.to_bytes().unwrap();
        let parsed = CredentialStore::parse(&bytes).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed.get("openrouter", "default").unwrap().secret.expose(),
            CANARY
        );
        assert_eq!(
            parsed.get("together", "work").unwrap().label.as_deref(),
            Some("Work")
        );
    }

    #[test]
    fn forget_leaves_the_other_entry_byte_identical() {
        let mut store = two_providers();
        let before = store.to_bytes().unwrap();
        let kept_before = entry_block(&before, "together/work");

        assert!(store.forget("openrouter", "default"));
        let after = store.to_bytes().unwrap();
        let kept_after = entry_block(&after, "together/work");

        assert_eq!(kept_before, kept_after);
        assert!(kept_after.contains("sk-together-second-key"));
        assert!(!String::from_utf8(after).unwrap().contains("MUSICANARY"));
        assert!(store.get("openrouter", "default").is_none());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn forgetting_an_absent_entry_reports_it() {
        let mut store = two_providers();
        assert!(!store.forget("openrouter", "other"));
        assert!(!store.forget("nobody", "default"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn forgetting_the_last_entry_empties_the_store() {
        let mut store = CredentialStore::new();
        store.insert(
            "openrouter",
            "default",
            Secret::new(CANARY.to_string()),
            None,
        );
        assert!(store.forget("openrouter", "default"));
        assert!(store.is_empty());
    }

    #[test]
    fn a_debug_print_of_the_whole_store_redacts_every_key() {
        let store = two_providers();
        let printed = format!("{store:?}");
        assert!(printed.contains("<redacted>"));
        assert!(!printed.contains("MUSICANARY"));
        assert!(!printed.contains("sk-together"));
    }

    #[test]
    fn an_unusable_key_is_refused_rather_than_mangled() {
        let mut store = CredentialStore::new();
        for (provider, lookup) in [
            ("Open Router", "default"),
            ("openrouter", "../escape"),
            ("openrouter", ""),
            ("", "default"),
            ("openrouter/nested", "default"),
        ] {
            assert!(
                !store.insert(provider, lookup, Secret::new(CANARY.to_string()), None),
                "{provider}/{lookup} was accepted"
            );
        }
        assert!(store.is_empty());
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let text = r#"{"schema":"musializer.assist-credentials/v1","entries":{
            "openrouter/default":{"secret":"sk","label":"x","note":"y"}}}"#;
        assert_eq!(
            CredentialStore::parse(text.as_bytes()).unwrap_err(),
            CredentialsError::Format
        );
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_store() {
        assert_eq!(
            CredentialStore::parse(b"{ broken").unwrap_err(),
            CredentialsError::Format
        );
    }

    #[test]
    fn a_foreign_schema_is_refused() {
        let text = r#"{"schema":"musializer.assist-credentials/v2","entries":{}}"#;
        assert_eq!(
            CredentialStore::parse(text.as_bytes()).unwrap_err(),
            CredentialsError::Schema("musializer.assist-credentials/v2".to_string())
        );
    }

    #[test]
    fn a_malformed_entry_key_in_a_file_is_refused() {
        let text = r#"{"schema":"musializer.assist-credentials/v1","entries":{
            "OpenRouter/Default":{"secret":"sk-or-v1-x"}}}"#;
        assert!(matches!(
            CredentialStore::parse(text.as_bytes()),
            Err(CredentialsError::Invalid(_))
        ));
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_parsed() {
        let bytes = vec![b' '; MAX_FILE_SIZE as usize + 1];
        assert_eq!(
            CredentialStore::parse(&bytes).unwrap_err(),
            CredentialsError::TooLarge
        );
    }

    #[test]
    fn two_writes_are_byte_identical() {
        let store = two_providers();
        assert_eq!(store.to_bytes().unwrap(), store.to_bytes().unwrap());
    }
}
