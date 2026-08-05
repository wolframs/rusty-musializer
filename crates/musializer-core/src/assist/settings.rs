//! `musializer.assist-settings/v1`: the non-secret preferences record.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §2. One versioned, atomically replaced
//! file in the per-user config directory; the filesystem half is
//! `musializer_runtime::assist::files`.
//!
//! Three properties are load-bearing and each has a test:
//!
//! - **A corrupt file is an error, never a silent reset.** Same rule as
//!   `ui/preferences.rs`: the application can keep running on defaults but must
//!   refuse to overwrite evidence the user may be able to repair.
//! - **A default-valued block is not serialized.** So a settings file written
//!   before a field existed stays readable *and byte-comparable* — which is what
//!   makes "two writes of a default profile are byte-identical" a check rather
//!   than a hope.
//! - **No field can hold a secret.** `deny_unknown_fields` everywhere plus a
//!   credentials block whose only members are `mode`, `lookup_id`, `fingerprint`
//!   and `label` (E11).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::assist::contracts::{ContractId, FallbackPolicy, RouteType};
use crate::assist::secret::FINGERPRINT_LENGTH;

/// The schema token. A file carrying anything else is refused.
pub const SCHEMA: &str = "musializer.assist-settings/v1";

/// The size cap, applied before parsing. `ui/preferences.rs` uses 64 KiB for a
/// four-field record; this one carries a routing matrix per profile, so it gets
/// more room and still cannot make the process allocate without bound.
pub const MAX_FILE_SIZE: u64 = 256 * 1024;

/// The built-in profile id. It is not writable and must not appear in the file:
/// a stored profile with this id would shadow the built-in defaults that every
/// absent field inherits (§2).
pub const RECOMMENDED_PROFILE: &str = "recommended";

/// The longest identifier, label or path this record will accept. A bound, not
/// a guess — the point is that an over-long string fails at parse time rather
/// than surviving into a path or a request.
pub const MAX_STRING_BYTES: usize = 512;

/// How many profiles a file may carry.
pub const MAX_PROFILES: usize = 64;

/// How many entries a provider constraint list may carry.
pub const MAX_PROVIDER_LIST: usize = 64;

/// What went wrong with a settings record.
///
/// Deliberately more specific than `ui/preferences.rs`'s `Format`, because these
/// reach a dialog that has to tell the user what to fix.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("the assist settings file is larger than the {MAX_FILE_SIZE} byte cap")]
    TooLarge,
    #[error("the assist settings file is not valid JSON for this schema")]
    Format,
    #[error("the assist settings file declares schema {0:?}, not {SCHEMA}")]
    Schema(String),
    #[error("{0}")]
    Invalid(String),
}

/// Codex's reasoning effort. `codex` routes only (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

/// When a local runtime may separate a vocal stem before analysis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StemSeparation {
    #[default]
    Never,
    OnDemand,
    Always,
}

/// Where a provider credential comes from (§3). This is the *mode*, never the
/// secret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialMode {
    /// No credential configured.
    #[default]
    None,
    /// The `0600` credentials file.
    File,
    /// Held in process memory for this run only; nothing is written.
    Session,
    /// Imported once from the environment at startup, then removed from it.
    EnvImport,
}

/// OpenRouter provider-selection constraints. Never a secret (§2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Provider {
    /// Preferred provider slugs, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    pub allow_fallbacks: bool,
    pub zdr_required: bool,
    /// USD per 1M tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_prompt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_completion: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_price_audio: Option<f64>,
}

impl Provider {
    /// §2's contextual defaults: `allow_fallbacks` is false for any contract
    /// whose data leaves the machine, and `zdr_required` is true for any
    /// contract that can carry audio off it.
    ///
    /// Contextual rather than a constant because the defaults are a policy about
    /// the *contract*, and a single `Default::default()` would silently apply
    /// the strict answer to a local contract and the lax one to a remote.
    #[must_use]
    pub fn defaults_for(contract: ContractId) -> Self {
        Self {
            allow_fallbacks: !contract.leaves_machine(),
            zdr_required: contract.max_boundary().rank() >= 2,
            ..Self::default()
        }
    }
}

/// One contract's resolved route (§2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub contract: ContractId,
    pub route_type: RouteType,
    /// `"whisper.cpp" | "mms-ctc" | "qwen3-fa" | "codex" | "openrouter"`.
    pub runtime_id: String,
    /// A provider model slug, or a local model identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Local weights, resolved against `local_runtimes.models_dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub fallback: FallbackPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
}

/// A named set of routes. Sparse: an absent contract inherits the built-in
/// `recommended` profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub routes: BTreeMap<ContractId, Route>,
}

/// Local runtime configuration (§2). Every field overrides an environment
/// variable the helpers already read, so the dialog and the shell agree.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LocalRuntimes {
    /// Operator decision: default `<install dir>/models/`, fallback a
    /// `musializer` directory in the user's home; always shown, never a location
    /// the user was not shown. Empty means "resolve the default at startup".
    #[serde(skip_serializing_if = "String::is_empty")]
    pub models_dir: String,
    /// Overrides `MUSIALIZER_WHISPER_BIN`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whisper_bin: Option<String>,
    /// Overrides `MUSIALIZER_WHISPER_MODEL`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whisper_model: Option<String>,
    /// Overrides `MUSIALIZER_ALIGN_PYTHON`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_python: Option<String>,
    /// An explicit path to the `codex` executable.
    ///
    /// Discovery finds it on `PATH`, in a documented list of install locations,
    /// and finally through the login shell — see
    /// `musializer_runtime::assist::discover`. This is the escape hatch for the
    /// installation none of that reaches, and it follows the same rule as
    /// `whisper_bin`: set-but-missing is a **loud failure**, never a silent
    /// fallback to whatever discovery would have found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_bin: Option<String>,
    pub prefer_gpu: bool,
    pub stem_separation: StemSeparation,
}

impl LocalRuntimes {
    #[must_use]
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Catalog fetch policy (§2). `network_allowed` defaults to false, so opening
/// the dialog cannot make a request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Catalog {
    pub network_allowed: bool,
    pub refresh_on_open: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub last_filters: BTreeMap<String, String>,
    /// RFC 3339; display only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh_utc: Option<String>,
    pub show_experimental: bool,
}

impl Catalog {
    #[must_use]
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// What is known about a provider credential *without* holding one (§3, E11).
///
/// Every field here is safe to display, log and write. There is deliberately no
/// field a secret could occupy, and `deny_unknown_fields` means a file that adds
/// one is refused rather than round-tripped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CredentialRecord {
    pub mode: CredentialMode,
    /// An opaque account label, e.g. `"default"`. Never the secret.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub lookup_id: String,
    /// First 8 hex of `sha256(secret)`. Never any key characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The provider-returned label from the last successful `Test`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The credentials *metadata* block (§2). One record per provider.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Credentials {
    #[serde(skip_serializing_if = "CredentialRecord::is_default")]
    pub openrouter: CredentialRecord,
}

impl CredentialRecord {
    #[must_use]
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

impl Credentials {
    #[must_use]
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// The whole record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistSettings {
    pub schema: String,
    /// The active profile id. `"recommended"` is built in and unwritable.
    pub active_profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Profile>,
    #[serde(default, skip_serializing_if = "LocalRuntimes::is_default")]
    pub local_runtimes: LocalRuntimes,
    #[serde(default, skip_serializing_if = "Catalog::is_default")]
    pub catalog: Catalog,
    #[serde(default, skip_serializing_if = "Credentials::is_default")]
    pub credentials: Credentials,
}

impl Default for AssistSettings {
    fn default() -> Self {
        Self {
            schema: SCHEMA.to_string(),
            active_profile: RECOMMENDED_PROFILE.to_string(),
            profiles: Vec::new(),
            local_runtimes: LocalRuntimes::default(),
            catalog: Catalog::default(),
            credentials: Credentials::default(),
        }
    }
}

impl AssistSettings {
    /// Parses and validates. The size cap is applied here rather than only in
    /// the loader, so a caller holding bytes from anywhere gets the same refusal.
    pub fn parse(bytes: &[u8]) -> Result<Self, SettingsError> {
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(SettingsError::TooLarge);
        }
        let settings: Self = serde_json::from_slice(bytes).map_err(|_| SettingsError::Format)?;
        if settings.schema != SCHEMA {
            return Err(SettingsError::Schema(settings.schema));
        }
        settings.validate()?;
        Ok(settings)
    }

    /// Serializes. Deterministic by construction: struct field order is fixed,
    /// every map is a `BTreeMap`, and a default block is skipped — so two writes
    /// of the same value are byte-identical.
    pub fn to_bytes(&self) -> Result<Vec<u8>, SettingsError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|_| SettingsError::Format)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(SettingsError::TooLarge);
        }
        Ok(bytes)
    }

    /// The profile the given id resolves to, if it is stored. `recommended` is
    /// built in and never stored, so it resolves to `None` and the caller
    /// inherits the built-in defaults.
    #[must_use]
    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// Everything §1 and §2 say about a well-formed record, in one place.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema != SCHEMA {
            return Err(SettingsError::Schema(self.schema.clone()));
        }
        check_identifier("active_profile", &self.active_profile)?;
        if self.profiles.len() > MAX_PROFILES {
            return Err(SettingsError::Invalid(format!(
                "a settings file may carry at most {MAX_PROFILES} profiles"
            )));
        }
        let mut seen = BTreeSet::new();
        for profile in &self.profiles {
            check_identifier("profile id", &profile.id)?;
            check_string("profile label", &profile.label)?;
            if profile.id == RECOMMENDED_PROFILE {
                return Err(SettingsError::Invalid(
                    "the recommended profile is built in and cannot be stored".to_string(),
                ));
            }
            if !seen.insert(profile.id.as_str()) {
                return Err(SettingsError::Invalid(format!(
                    "profile id {:?} appears twice",
                    profile.id
                )));
            }
            for (key, route) in &profile.routes {
                validate_route(*key, route)?;
            }
        }
        if self.active_profile != RECOMMENDED_PROFILE
            && self.profile(&self.active_profile).is_none()
        {
            return Err(SettingsError::Invalid(format!(
                "active_profile {:?} names no stored profile",
                self.active_profile
            )));
        }
        validate_local_runtimes(&self.local_runtimes)?;
        validate_catalog(&self.catalog)?;
        validate_credential_record(&self.credentials.openrouter)?;
        Ok(())
    }
}

fn validate_route(key: ContractId, route: &Route) -> Result<(), SettingsError> {
    if route.contract != key {
        return Err(SettingsError::Invalid(format!(
            "route stored under {} declares {}",
            key.token(),
            route.contract.token()
        )));
    }
    if key.is_locked() {
        return Err(SettingsError::Invalid(format!(
            "{} is locked and is not user-routable",
            key.token()
        )));
    }
    if !key.eligible_route_types().contains(&route.route_type) {
        return Err(SettingsError::Invalid(format!(
            "{} cannot be served by a {} route",
            key.token(),
            route.route_type.token()
        )));
    }
    if route.route_type.minimum_boundary().rank() > key.max_boundary().rank() {
        return Err(SettingsError::Invalid(format!(
            "{} caps at {} and a {} route cannot stay inside it",
            key.token(),
            key.max_boundary().token(),
            route.route_type.token()
        )));
    }
    if !key.allowed_fallbacks().contains(&route.fallback) {
        return Err(SettingsError::Invalid(format!(
            "{} does not allow the {} fallback policy",
            key.token(),
            route.fallback.token()
        )));
    }
    check_identifier("runtime_id", &route.runtime_id)?;
    if let Some(model_id) = &route.model_id {
        check_string("model_id", model_id)?;
    }
    if let Some(model_path) = &route.model_path {
        check_string("model_path", model_path)?;
    }
    if route.reasoning_effort.is_some() && route.route_type != RouteType::Codex {
        return Err(SettingsError::Invalid(
            "reasoning_effort belongs to codex routes only".to_string(),
        ));
    }
    match (&route.provider, route.route_type) {
        (Some(provider), RouteType::OpenRouter) => validate_provider(provider),
        (Some(_), other) => Err(SettingsError::Invalid(format!(
            "provider constraints belong to openrouter routes, not {}",
            other.token()
        ))),
        (None, _) => Ok(()),
    }
}

fn validate_provider(provider: &Provider) -> Result<(), SettingsError> {
    for (name, list) in [
        ("order", &provider.order),
        ("only", &provider.only),
        ("ignore", &provider.ignore),
    ] {
        if list.len() > MAX_PROVIDER_LIST {
            return Err(SettingsError::Invalid(format!(
                "provider {name} may carry at most {MAX_PROVIDER_LIST} slugs"
            )));
        }
        for slug in list {
            check_identifier(name, slug)?;
        }
    }
    for (name, price) in [
        ("max_price_prompt", provider.max_price_prompt),
        ("max_price_completion", provider.max_price_completion),
        ("max_price_audio", provider.max_price_audio),
    ] {
        if let Some(price) = price {
            if !price.is_finite() || price < 0.0 {
                return Err(SettingsError::Invalid(format!(
                    "provider {name} must be a finite, non-negative price"
                )));
            }
        }
    }
    Ok(())
}

fn validate_local_runtimes(runtimes: &LocalRuntimes) -> Result<(), SettingsError> {
    if !runtimes.models_dir.is_empty() {
        check_string("models_dir", &runtimes.models_dir)?;
    }
    for (name, value) in [
        ("whisper_bin", &runtimes.whisper_bin),
        ("whisper_model", &runtimes.whisper_model),
        ("align_python", &runtimes.align_python),
        ("codex_bin", &runtimes.codex_bin),
    ] {
        if let Some(value) = value {
            check_string(name, value)?;
        }
    }
    Ok(())
}

fn validate_catalog(catalog: &Catalog) -> Result<(), SettingsError> {
    if catalog.last_filters.len() > MAX_PROVIDER_LIST {
        return Err(SettingsError::Invalid(
            "catalog last_filters carries too many entries".to_string(),
        ));
    }
    for (key, value) in &catalog.last_filters {
        check_identifier("catalog filter key", key)?;
        check_string("catalog filter value", value)?;
    }
    if let Some(stamp) = &catalog.last_refresh_utc {
        check_string("last_refresh_utc", stamp)?;
    }
    Ok(())
}

fn validate_credential_record(record: &CredentialRecord) -> Result<(), SettingsError> {
    if !record.lookup_id.is_empty() {
        check_identifier("lookup_id", &record.lookup_id)?;
    }
    if let Some(fingerprint) = &record.fingerprint {
        let well_formed = fingerprint.len() == FINGERPRINT_LENGTH
            && fingerprint
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, 'a'..='f'));
        if !well_formed {
            return Err(SettingsError::Invalid(format!(
                "fingerprint must be {FINGERPRINT_LENGTH} lowercase hex characters"
            )));
        }
    }
    if let Some(label) = &record.label {
        check_string("credential label", label)?;
    }
    if record.mode == CredentialMode::None && record.fingerprint.is_some() {
        return Err(SettingsError::Invalid(
            "a fingerprint without a credential mode describes a key that is not configured"
                .to_string(),
        ));
    }
    Ok(())
}

fn check_string(name: &str, value: &str) -> Result<(), SettingsError> {
    if value.is_empty() {
        return Err(SettingsError::Invalid(format!("{name} must not be empty")));
    }
    if value.len() > MAX_STRING_BYTES {
        return Err(SettingsError::Invalid(format!(
            "{name} is longer than {MAX_STRING_BYTES} bytes"
        )));
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(SettingsError::Invalid(format!(
            "{name} must not contain control characters"
        )));
    }
    Ok(())
}

/// Identifiers become map keys, cache keys and request fields, so they are held
/// to an explicit charset rather than only to "no control characters". Catalog
/// strings are untrusted display data and must never become a path or a shell
/// fragment (E6); this is where that stops being a comment.
fn check_identifier(name: &str, value: &str) -> Result<(), SettingsError> {
    check_string(name, value)?;
    let acceptable = value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/' | ':')
    });
    if !acceptable {
        return Err(SettingsError::Invalid(format!(
            "{name} may only contain letters, digits, and - _ . / :"
        )));
    }
    // `/` has to be allowed (`provider/lookup_id`, `vendor/model`), which is
    // exactly what makes `..` and a leading separator worth refusing here rather
    // than at whatever call site first joins one onto a path.
    if value.contains("..") || value.starts_with('/') || value.starts_with('.') {
        return Err(SettingsError::Invalid(format!(
            "{name} must not be a relative or absolute path fragment"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::contracts::Boundary;

    fn openrouter_route(contract: ContractId) -> Route {
        Route {
            contract,
            route_type: RouteType::OpenRouter,
            runtime_id: "openrouter".to_string(),
            model_id: Some("xiaomi/mimo-v2.5".to_string()),
            model_path: None,
            reasoning_effort: None,
            fallback: FallbackPolicy::Ask,
            provider: Some(Provider::defaults_for(contract)),
        }
    }

    fn populated() -> AssistSettings {
        let mut routes = BTreeMap::new();
        routes.insert(ContractId::Semantic, openrouter_route(ContractId::Semantic));
        routes.insert(
            ContractId::Align,
            Route {
                contract: ContractId::Align,
                route_type: RouteType::LocalProc,
                runtime_id: "mms-ctc".to_string(),
                model_id: None,
                model_path: Some("mms-ctc/model.bin".to_string()),
                reasoning_effort: None,
                fallback: FallbackPolicy::LocalOnly,
                provider: None,
            },
        );
        routes.insert(
            ContractId::Wording,
            Route {
                contract: ContractId::Wording,
                route_type: RouteType::Codex,
                runtime_id: "codex".to_string(),
                model_id: None,
                model_path: None,
                reasoning_effort: Some(ReasoningEffort::Medium),
                fallback: FallbackPolicy::None,
                provider: None,
            },
        );
        AssistSettings {
            profiles: vec![Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes,
            }],
            active_profile: "studio".to_string(),
            local_runtimes: LocalRuntimes {
                models_dir: "/home/example/.local/share/musializer/models".to_string(),
                whisper_bin: Some("/usr/bin/whisper-cli".to_string()),
                prefer_gpu: true,
                stem_separation: StemSeparation::OnDemand,
                ..LocalRuntimes::default()
            },
            catalog: Catalog {
                network_allowed: true,
                last_filters: BTreeMap::from([
                    ("input_modalities".to_string(), "audio".to_string()),
                    ("output_modalities".to_string(), "text".to_string()),
                ]),
                last_refresh_utc: Some("2026-08-04T12:00:00Z".to_string()),
                ..Catalog::default()
            },
            credentials: Credentials {
                openrouter: CredentialRecord {
                    mode: CredentialMode::File,
                    lookup_id: "default".to_string(),
                    fingerprint: Some("0a1b2c3d".to_string()),
                    label: Some("Personal".to_string()),
                },
            },
            ..AssistSettings::default()
        }
    }

    #[test]
    fn a_populated_record_round_trips() {
        let settings = populated();
        let bytes = settings.to_bytes().unwrap();
        assert_eq!(AssistSettings::parse(&bytes).unwrap(), settings);
    }

    #[test]
    fn two_writes_of_the_default_profile_are_byte_identical() {
        let first = AssistSettings::default().to_bytes().unwrap();
        let second = AssistSettings::default().to_bytes().unwrap();
        assert_eq!(first, second);
        // And the default really is minimal: only the two required fields, so a
        // file written before any optional block existed stays comparable.
        let text = String::from_utf8(first).unwrap();
        assert!(text.contains("\"schema\""));
        assert!(text.contains("\"active_profile\""));
        assert!(!text.contains("local_runtimes"));
        assert!(!text.contains("catalog"));
        assert!(!text.contains("credentials"));
        assert!(!text.contains("profiles"));
    }

    #[test]
    fn a_populated_record_writes_the_same_bytes_twice() {
        let settings = populated();
        assert_eq!(settings.to_bytes().unwrap(), settings.to_bytes().unwrap());
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let text = r#"{"schema":"musializer.assist-settings/v1","active_profile":"recommended","future_field":1}"#;
        assert_eq!(
            AssistSettings::parse(text.as_bytes()),
            Err(SettingsError::Format)
        );
    }

    #[test]
    fn an_unknown_field_inside_a_route_is_refused() {
        let text = r#"{
            "schema":"musializer.assist-settings/v1",
            "active_profile":"studio",
            "profiles":[{"id":"studio","label":"Studio","routes":{
                "TC-SEMANTIC":{"contract":"TC-SEMANTIC","route_type":"openrouter",
                "runtime_id":"openrouter","fallback":"ask","api_key":"sk-or-v1-leak"}}}]
        }"#;
        assert_eq!(
            AssistSettings::parse(text.as_bytes()),
            Err(SettingsError::Format)
        );
    }

    /// E11: `assist.json` carries `lookup_id` and `fingerprint` only. The test is
    /// two-sided — the four documented members must round-trip, and anything
    /// that could hold a key must be refused.
    #[test]
    fn the_credentials_block_has_no_field_a_secret_could_occupy() {
        let record = serde_json::to_value(CredentialRecord {
            mode: CredentialMode::File,
            lookup_id: "default".to_string(),
            fingerprint: Some("0a1b2c3d".to_string()),
            label: Some("Personal".to_string()),
        })
        .unwrap();
        let members: BTreeSet<&str> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            members,
            BTreeSet::from(["mode", "lookup_id", "fingerprint", "label"])
        );
        for planted in [
            r#"{"mode":"file","api_key":"sk-or-v1-MUSICANARY7Q4X2ZK9"}"#,
            r#"{"mode":"file","secret":"sk-or-v1-MUSICANARY7Q4X2ZK9"}"#,
            r#"{"mode":"file","token":"sk-or-v1-MUSICANARY7Q4X2ZK9"}"#,
        ] {
            assert!(serde_json::from_str::<CredentialRecord>(planted).is_err());
        }
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_parsed() {
        let mut bytes = AssistSettings::default().to_bytes().unwrap();
        bytes.resize(MAX_FILE_SIZE as usize + 1, b' ');
        assert_eq!(AssistSettings::parse(&bytes), Err(SettingsError::TooLarge));
    }

    #[test]
    fn a_corrupt_file_is_a_format_error_not_an_empty_record() {
        assert_eq!(
            AssistSettings::parse(b"{ broken"),
            Err(SettingsError::Format)
        );
        assert_eq!(AssistSettings::parse(b""), Err(SettingsError::Format));
        assert_eq!(AssistSettings::parse(b"null"), Err(SettingsError::Format));
    }

    #[test]
    fn a_foreign_schema_names_itself_in_the_error() {
        let text = r#"{"schema":"musializer.assist-settings/v2","active_profile":"recommended"}"#;
        assert_eq!(
            AssistSettings::parse(text.as_bytes()),
            Err(SettingsError::Schema(
                "musializer.assist-settings/v2".to_string()
            ))
        );
    }

    #[test]
    fn a_route_may_not_exceed_its_contract_boundary() {
        let mut settings = AssistSettings::default();
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Align, openrouter_route(ContractId::Align))]),
        });
        settings.active_profile = "studio".to_string();
        assert_eq!(ContractId::Align.max_boundary(), Boundary::LocalOnly);
        let error = settings.validate().unwrap_err();
        assert!(matches!(error, SettingsError::Invalid(_)), "{error:?}");
    }

    #[test]
    fn a_locked_contract_may_not_be_stored_as_a_route() {
        let route = Route {
            contract: ContractId::Measured,
            route_type: RouteType::Builtin,
            runtime_id: "builtin".to_string(),
            model_id: None,
            model_path: None,
            reasoning_effort: None,
            fallback: FallbackPolicy::None,
            provider: None,
        };
        let settings = AssistSettings {
            active_profile: "studio".to_string(),
            profiles: vec![Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes: BTreeMap::from([(ContractId::Measured, route)]),
            }],
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn a_route_stored_under_the_wrong_key_is_refused() {
        let settings = AssistSettings {
            active_profile: "studio".to_string(),
            profiles: vec![Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes: BTreeMap::from([(
                    ContractId::Plan,
                    openrouter_route(ContractId::Semantic),
                )]),
            }],
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn a_disallowed_fallback_policy_is_refused() {
        let mut route = openrouter_route(ContractId::Semantic);
        route.fallback = FallbackPolicy::SameBoundary;
        assert!(!ContractId::Semantic
            .allowed_fallbacks()
            .contains(&FallbackPolicy::SameBoundary));
        let settings = AssistSettings {
            active_profile: "studio".to_string(),
            profiles: vec![Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes: BTreeMap::from([(ContractId::Semantic, route)]),
            }],
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn the_built_in_profile_cannot_be_stored() {
        let settings = AssistSettings {
            profiles: vec![Profile {
                id: RECOMMENDED_PROFILE.to_string(),
                label: "Recommended".to_string(),
                routes: BTreeMap::new(),
            }],
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn an_active_profile_that_names_nothing_is_refused() {
        let settings = AssistSettings {
            active_profile: "missing".to_string(),
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn provider_defaults_are_strict_exactly_where_the_data_leaves() {
        let local = Provider::defaults_for(ContractId::Align);
        assert!(local.allow_fallbacks);
        assert!(!local.zdr_required);
        let text = Provider::defaults_for(ContractId::Wording);
        assert!(!text.allow_fallbacks);
        assert!(!text.zdr_required);
        let audio = Provider::defaults_for(ContractId::Semantic);
        assert!(!audio.allow_fallbacks);
        assert!(audio.zdr_required);
    }

    #[test]
    fn a_malformed_fingerprint_is_refused() {
        for fingerprint in ["0A1B2C3D", "0a1b2c3", "0a1b2c3de", "zzzzzzzz"] {
            let settings = AssistSettings {
                credentials: Credentials {
                    openrouter: CredentialRecord {
                        mode: CredentialMode::File,
                        lookup_id: "default".to_string(),
                        fingerprint: Some(fingerprint.to_string()),
                        label: None,
                    },
                },
                ..AssistSettings::default()
            };
            assert!(settings.validate().is_err(), "{fingerprint} was accepted");
        }
    }

    #[test]
    fn a_catalog_string_cannot_smuggle_a_path_fragment_into_an_identifier() {
        let settings = AssistSettings {
            catalog: Catalog {
                last_filters: BTreeMap::from([("../../etc".to_string(), "audio".to_string())]),
                ..Catalog::default()
            },
            ..AssistSettings::default()
        };
        assert!(settings.validate().is_err());
    }
}
