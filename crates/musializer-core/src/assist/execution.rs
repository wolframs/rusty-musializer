//! Route resolution and the `musializer.assist-execution/v1` snapshot.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §5 (fallback invariants) and §6 (the
//! execution snapshot). Everything here is pure: no filesystem, no clock, no
//! process. `musializer_runtime::assist::plan` supplies the four impure facts
//! (the settings file, the credential, the caches, the wall clock) and this
//! module decides what they mean.
//!
//! ## Why the resolver lives in this crate rather than beside the dialog
//!
//! It was written for `ui/assist_settings.rs`'s dry-run summary — "what the
//! **next** job would use" — and P4 needs the *same* answer at Start. Two
//! copies of a routing table is exactly the drift `docs/ASSIST_PIPELINE.md`
//! warns about, and one of them would be the one a snapshot records. So the
//! resolver moved down here and the dialog re-exports it; the dry-run summary
//! and the execution snapshot are now the same function called twice.
//!
//! ## The three things that are load-bearing
//!
//! - **A job snapshots its route graph once** (§5 invariant 3). [`resolve`] is
//!   called at Start and the result is immutable. Nothing re-runs the resolver
//!   against current settings afterwards, which is why [`ExecutionSnapshot`]
//!   has no method that takes `&AssistSettings`.
//! - **Nothing raises a boundary automatically** (§5 invariant 1). The only
//!   policy that could is `ask`, and this build has no way to pause a running
//!   job for an answer — so `ask` is resolved to `none` **at Start**, recorded
//!   as `none`, and named in the confirmation. That is the implemented
//!   semantics, not a silent approximation; [`AppliedFallback`] carries both
//!   the stored policy and the applied one so the confirmation can say which.
//! - **A constraint that leaves no endpoint blocks before anything spawns**
//!   (§5 invariant 4). [`preflight`] is that check, and it names the constraint
//!   rather than reporting a generic failure.

use serde::{Deserialize, Serialize};

use crate::assist::contracts::{Boundary, ContractId, FallbackPolicy, RouteType};
use crate::assist::settings::{
    AssistSettings, Provider, ReasoningEffort, Route, RECOMMENDED_PROFILE,
    SCHEMA as SETTINGS_SCHEMA,
};
use crate::assist::suitability::{self, AudioScope};

/// The snapshot schema token (§6).
pub const SNAPSHOT_SCHEMA: &str = "musializer.assist-execution/v1";

/// The label a Codex route shows when discovery never ran or failed.
///
/// §5 rule 6: "Codex discovery failure preserves `Codex default`. Never a
/// guessed model id." The string is exactly the one
/// `tools/codex_model_discovery.py` uses.
pub const CODEX_DEFAULT_LABEL: &str = "Codex default";

/// The three "first missing piece" sentences, named so the badge, its tooltip,
/// the preflight refusal and the tests cannot spell them differently.
pub const NO_KEY: &str = "No key";
pub const NO_MODEL: &str = "No model chosen";
pub const NO_ENDPOINT: &str = "No eligible endpoint";

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix timestamp.
///
/// Pure and hand-rolled for the reason the rest of this crate is: `chrono` is
/// not a dependency and one civil-date conversion is not worth becoming one.
/// This is Howard Hinnant's `civil_from_days`, the exact inverse of
/// `ui/assist_settings.rs::parse_rfc3339_utc`, and the two are pinned against
/// each other by that module's test vectors.
#[must_use]
pub fn format_rfc3339_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3_600,
        (time % 3_600) / 60,
        time % 60,
    )
}

// ---------------------------------------------------------------------------
// The recommended profile, and route resolution
// ---------------------------------------------------------------------------

/// The built-in `recommended` profile (§2: "`recommended` is built in and
/// unwritable").
///
/// Seeded **only** from evidence this repository holds, the same rule
/// `assist::suitability` follows: `mms-ctc` for `TC-ALIGN` is the benchmarked
/// aligner, `whisper.cpp` is the coarse lane the production path already runs,
/// `xiaomi/mimo-v2.5` is what `tools/mimo_openrouter.py` uses, and the planner is
/// the deterministic one.
///
/// `TC-VERIFY` deliberately has **no** recommended route. Independent timing
/// verification was never benchmarked here, and inventing a default for it would
/// be the invented field the honesty rule forbids — the dialog says "no
/// recommended route" and offers the eligible ones.
#[must_use]
pub fn recommended_route(contract: ContractId) -> Option<Route> {
    let route = |route_type: RouteType, runtime: &str, model: Option<&str>| Route {
        contract,
        route_type,
        runtime_id: runtime.to_string(),
        model_id: model.map(str::to_string),
        model_path: None,
        reasoning_effort: (route_type == RouteType::Codex).then_some(ReasoningEffort::Medium),
        fallback: FallbackPolicy::None,
        provider: (route_type == RouteType::OpenRouter).then(|| Provider::defaults_for(contract)),
    };
    match contract {
        ContractId::Measured => Some(route(RouteType::Builtin, "builtin-analyzer", None)),
        ContractId::Coarse => Some(route(
            RouteType::LocalProc,
            "whisper.cpp",
            Some("whisper.cpp"),
        )),
        ContractId::Align => Some(route(RouteType::LocalProc, "mms-ctc", Some("mms-ctc"))),
        ContractId::Wording => Some(route(RouteType::Codex, "codex", None)),
        ContractId::Semantic => Some(route(
            RouteType::OpenRouter,
            "openrouter",
            Some("xiaomi/mimo-v2.5"),
        )),
        ContractId::Plan => Some(route(RouteType::Builtin, "builtin-planner", None)),
        ContractId::Verify => None,
    }
}

/// Where a resolved route came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteOrigin {
    /// The built-in `recommended` profile.
    Recommended,
    /// A per-task override stored in the active profile.
    Override,
    /// Neither: the contract has no recommended route and none was chosen.
    Unrouted,
}

impl RouteOrigin {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            RouteOrigin::Recommended => "recommended",
            RouteOrigin::Override => "override",
            RouteOrigin::Unrouted => "unrouted",
        }
    }
}

/// One contract's resolved route: what the **next** job would use.
#[derive(Clone, Debug)]
pub struct ResolvedRoute {
    pub contract: ContractId,
    pub route: Option<Route>,
    pub origin: RouteOrigin,
}

impl ResolvedRoute {
    /// The model identity to display. An absent `model_id` on a Codex route is
    /// `Codex default` — the documented fallback, not a guess (§5 rule 6). A
    /// local runtime without an explicit weights override names the runtime
    /// family: the helper resolves and records the concrete model file. Calling
    /// that state "not chosen" contradicted the job that immediately ran it.
    #[must_use]
    pub fn model_label(&self) -> String {
        let Some(route) = &self.route else {
            return "\u{2014}".to_string();
        };
        match (&route.model_id, route.route_type) {
            (Some(id), _) => id.clone(),
            (None, RouteType::Codex) => CODEX_DEFAULT_LABEL.to_string(),
            (None, RouteType::Builtin | RouteType::LocalProc) => route.runtime_id.clone(),
            (None, _) => "not chosen".to_string(),
        }
    }

    #[must_use]
    pub fn route_label(&self) -> String {
        match &self.route {
            Some(route) => format!("{} / {}", route.route_type.token(), route.runtime_id),
            None => "no route".to_string(),
        }
    }

    /// The boundary the resolved route would actually operate at.
    ///
    /// Two halves, and both matter. A `builtin` or `local-proc` route opens no
    /// socket, so it is `local-only` whatever the contract's ceiling is —
    /// telling a user that a local Whisper lane sends audio off the machine
    /// would be the worst kind of wrong. A remote route reaches the contract's
    /// ceiling exactly, because the ceiling *is* what that contract's inputs
    /// are: §1 gives `TC-WORDING` bounded JSON and `TC-SEMANTIC` complete audio,
    /// so the route type's own minimum (`text-leaves-machine` for both) would
    /// understate the second one.
    #[must_use]
    pub fn boundary(&self) -> Boundary {
        match &self.route {
            Some(route) if route.route_type.minimum_boundary().rank() == 0 => Boundary::LocalOnly,
            Some(_) => self.contract.max_boundary(),
            None => Boundary::LocalOnly,
        }
    }

    /// Whether this route opens a socket at all.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        self.route
            .as_ref()
            .is_some_and(|route| route.route_type.minimum_boundary().rank() > 0)
    }

    /// The route as one greppable token for the report line.
    #[must_use]
    pub fn compact(&self) -> String {
        match &self.route {
            Some(route) => format!("{}/{}", route.route_type.token(), route.runtime_id),
            None => "no-route".to_string(),
        }
    }
}

/// Applies §2's inheritance: a stored route for the active profile wins, an
/// absent one inherits the built-in `recommended` profile.
#[must_use]
pub fn resolve_route(settings: &AssistSettings, contract: ContractId) -> ResolvedRoute {
    if let Some(stored) = settings
        .profile(&settings.active_profile)
        .and_then(|profile| profile.routes.get(&contract))
    {
        return ResolvedRoute {
            contract,
            route: Some(stored.clone()),
            origin: RouteOrigin::Override,
        };
    }
    match recommended_route(contract) {
        Some(route) => ResolvedRoute {
            contract,
            route: Some(route),
            origin: RouteOrigin::Recommended,
        },
        None => ResolvedRoute {
            contract,
            route: None,
            origin: RouteOrigin::Unrouted,
        },
    }
}

// ---------------------------------------------------------------------------
// What a workflow composes
// ---------------------------------------------------------------------------

/// The four workflow buttons, as contract-composition inputs.
///
/// Spelled here rather than reused from `ui::assist_ui_state::AssistMode`
/// because that type is a *panel* state and this is the pipeline's own shape;
/// the two agree today and a test pins that they compose what
/// `tools/external_analysis.py::run_assist` actually runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowKind {
    Lyrics,
    Sections,
    Mimo,
    All,
}

impl WorkflowKind {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Lyrics => "lyrics",
            Self::Sections => "sections",
            Self::Mimo => "mimo",
            Self::All => "all",
        }
    }

    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        [Self::Lyrics, Self::Sections, Self::Mimo, Self::All]
            .into_iter()
            .find(|kind| kind.token() == token)
    }
}

/// The contracts one workflow composes, in pipeline order.
///
/// Read off `run_assist`'s `actions` list rather than invented:
///
/// - `measured` and `plan` run in **every** mode, so `TC-MEASURED` and
///   `TC-PLAN` are always composed.
/// - `lyrics`/`all` add the Whisper evidence pass (`TC-COARSE`) and the
///   acoustic stage (`TC-ALIGN`).
/// - `TC-WORDING` is the Codex review, and the helper reaches it **only** when
///   no authored lyric text exists. `has_lyric_reference` is what this side
///   knows: an explicitly chosen sheet or a sibling `<stem>.lyrics.txt`. It
///   cannot see an embedded tag, so a job with neither still composes
///   `TC-WORDING` — which is the safe direction, because the confirmation then
///   names a route that *may* run rather than hiding one that did.
/// - `mimo`/`all` add `TC-SEMANTIC`.
/// - `TC-VERIFY` is composed by nothing: independent timing verification has no
///   lane in the helper, and offering a route for a stage that cannot run is
///   the invented capability the honesty rule forbids.
#[must_use]
pub fn composed_contracts(kind: WorkflowKind, has_lyric_reference: bool) -> Vec<ContractId> {
    let mut contracts = vec![ContractId::Measured];
    if matches!(kind, WorkflowKind::Lyrics | WorkflowKind::All) {
        contracts.push(ContractId::Coarse);
        if !has_lyric_reference {
            contracts.push(ContractId::Wording);
        }
        contracts.push(ContractId::Align);
    }
    if matches!(kind, WorkflowKind::Mimo | WorkflowKind::All) {
        contracts.push(ContractId::Semantic);
    }
    contracts.push(ContractId::Plan);
    contracts
}

// ---------------------------------------------------------------------------
// Fallback policy, as applied
// ---------------------------------------------------------------------------

/// What a route's stored fallback policy became for this job.
///
/// `ask` has no implementation in this build: pausing a running job to show a
/// substitute route and wait for an answer needs a job state nothing here can
/// reach, and a half-built pause that silently continued would be the one thing
/// §5 invariant 1 forbids. So `ask` resolves to `none` **at Start**, which
/// cannot raise a boundary by construction, and the confirmation says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppliedFallback {
    /// What the settings file stores.
    pub stored: FallbackPolicy,
    /// What this job will do.
    pub applied: FallbackPolicy,
}

impl AppliedFallback {
    #[must_use]
    pub fn of(stored: FallbackPolicy) -> Self {
        Self {
            stored,
            applied: match stored {
                FallbackPolicy::Ask => FallbackPolicy::None,
                other => other,
            },
        }
    }

    /// Whether the applied policy differs from the stored one, which is the
    /// only case the confirmation has anything extra to say about.
    #[must_use]
    pub fn was_downgraded(self) -> bool {
        self.stored != self.applied
    }
}

/// Whether **any** applied policy in a graph could move data to a higher
/// boundary rank. It cannot, and this function is how the confirmation gets to
/// say so as a measurement rather than a promise (§5 invariant 1).
#[must_use]
pub fn any_fallback_can_raise_boundary(contracts: &[ContractSnapshot]) -> bool {
    contracts.iter().any(|entry| {
        let ladder = [
            Boundary::LocalOnly,
            Boundary::TextLeavesMachine,
            Boundary::AudioLeavesMachine,
        ];
        ladder.into_iter().any(|candidate| {
            candidate.rank() > entry.boundary_applied.rank()
                && entry
                    .fallback_policy
                    .permits_automatic_substitute(entry.boundary_applied, candidate)
        })
    })
}

// ---------------------------------------------------------------------------
// The snapshot (§6)
// ---------------------------------------------------------------------------

/// One contract's row in the execution snapshot (§6).
///
/// Every field is serialized, including the absent ones as `null`: the snapshot
/// is provenance, not a preference record, so "the field was not written" and
/// "the field had no value" must not be the same picture. That is the opposite
/// of `assist.json`'s skip-if-default rule and the reason is the same one —
/// each record is read for a different question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSnapshot {
    pub contract: ContractId,
    pub route_type: RouteType,
    pub runtime_id: String,
    /// e.g. a whisper.cpp build or an aligner package version. Filled in by the
    /// runtime from the doctor report where one has been taken.
    pub runtime_version: Option<String>,
    /// The model that **actually ran**. At Start this is the resolved identity;
    /// the helper replaces it with what it observed — for OpenRouter the
    /// response's own `model` field, for Codex what `--model` was invoked with
    /// (§6). The file this side writes is never rewritten.
    pub model_id: String,
    pub model_sha256: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub boundary_applied: Boundary,
    /// True only where rank >= 1 and a user confirmed (§5 invariant 2).
    pub boundary_confirmed: bool,
    pub audio_scope: Option<AudioScope>,
    /// `[start, end]` seconds, when `audio_scope` is `excerpts`.
    pub excerpt_spans: Vec<[f64; 2]>,
    /// As sent, including `zdr`. `null` for a route that opens no socket.
    pub provider_constraints: Option<Provider>,
    pub provider_served: Option<String>,
    pub prompt_version: Option<String>,
    pub prompt_sha256: Option<String>,
    /// The output schema the response was validated against.
    pub schema_version: Option<String>,
    /// The policy **as applied**. `ask` never appears here: see
    /// [`AppliedFallback`].
    pub fallback_policy: FallbackPolicy,
    pub fallback_taken: bool,
    pub fallback_from: Option<String>,
}

impl ContractSnapshot {
    /// One greppable token for a report line and for the confirmation's own
    /// evidence.
    #[must_use]
    pub fn compact(&self) -> String {
        format!(
            "{}={}/{}[{}]{}",
            self.contract.token(),
            self.route_type.token(),
            self.model_id,
            self.boundary_applied.token(),
            if self.boundary_confirmed { "+" } else { "" },
        )
    }
}

/// The whole record (§6). Written once per job, before the helper starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSnapshot {
    pub snapshot_schema: String,
    /// The `assist.json` schema that resolved it.
    pub settings_schema: String,
    pub profile_id: String,
    /// RFC 3339.
    pub resolved_at_utc: String,
    pub contracts: Vec<ContractSnapshot>,
    /// Cache schema version plus fetch timestamp.
    pub catalog_revision: Option<String>,
    pub suitability_revision: Option<String>,
    pub credential_present: bool,
    /// `sha256(secret)[0..8]`; never the lookup label (E4).
    pub credential_fingerprint: Option<String>,
}

impl ExecutionSnapshot {
    /// Serializes deterministically: field order is declaration order and every
    /// map inside is a `BTreeMap`, so two writes of the same value are
    /// byte-identical. That is what makes "settings edited after Start left the
    /// running job's snapshot alone" a `cmp`, not an argument.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    #[must_use]
    pub fn contract(&self, contract: ContractId) -> Option<&ContractSnapshot> {
        self.contracts
            .iter()
            .find(|entry| entry.contract == contract)
    }

    /// Whether any contract in the graph opens a socket. This is the question
    /// that decides whether the child gets a credential at all (§4 E1).
    #[must_use]
    pub fn has_remote_route(&self) -> bool {
        self.contracts
            .iter()
            .any(|entry| entry.boundary_applied.rank() > 0)
    }

    /// Whether audio bytes leave the machine, which is the confirmation that
    /// costs a separate decision (§1).
    #[must_use]
    pub fn sends_audio_off_machine(&self) -> bool {
        self.contracts
            .iter()
            .any(|entry| entry.boundary_applied.rank() >= 2)
    }

    /// Whether this job may be handed a provider credential at all.
    ///
    /// Both halves are required and neither is redundant (§4 E1, §5 invariant
    /// 2): there must be a route that opens a socket, **and** every such route
    /// must carry this job's own confirmation. A graph that is entirely local
    /// has nothing to authorize, and an unconfirmed boundary is a request the
    /// user has not agreed to — either way the child's environment gets no key,
    /// which is the only thing that keeps a local-only job's helper unable to
    /// leak one however it is configured.
    #[must_use]
    pub fn authorizes_credential(&self) -> bool {
        let mut remote = false;
        for entry in &self.contracts {
            if entry.boundary_applied.rank() == 0 {
                continue;
            }
            remote = true;
            if !entry.boundary_confirmed {
                return false;
            }
        }
        remote
    }
}

/// The impure facts [`resolve`] needs, gathered by the runtime and handed in.
///
/// A struct rather than eight parameters so a caller cannot silently pass
/// `false` for `credential_present` while meaning "not looked yet" — every
/// field here has one meaning and the type says which.
#[derive(Clone, Debug, Default)]
pub struct ExecutionFacts {
    /// RFC 3339, from the wall clock at Start.
    pub resolved_at_utc: String,
    pub credential_present: bool,
    pub credential_fingerprint: Option<String>,
    /// Cache schema version plus fetch timestamp, or `None` when the catalog
    /// was never fetched — which is not the same as an empty catalog.
    pub catalog_revision: Option<String>,
    /// `(runtime key, version)` pairs from a doctor report, where one was taken.
    pub runtime_versions: Vec<(String, String)>,
    /// `(runtime key, model sha256)` pairs, same source.
    pub model_digests: Vec<(String, String)>,
    /// True once the user has confirmed this job's boundary. The confirmation
    /// step is what sets it, and §5 invariant 2 is why it is per job.
    pub boundary_confirmed: bool,
}

/// The composed contracts whose **stored** fallback policy is `ask`.
///
/// A snapshot never records `ask` — that is the invariant — so the confirmation
/// would otherwise have no way to say "you asked to be asked, and this build
/// cannot, so it is `none` for this job". This is the one place the stored side
/// is read for display, and it is a separate function rather than a field on the
/// snapshot precisely so the snapshot stays a record of what ran.
#[must_use]
pub fn stored_ask_contracts(
    settings: &AssistSettings,
    kind: WorkflowKind,
    has_lyric_reference: bool,
) -> Vec<ContractId> {
    composed_contracts(kind, has_lyric_reference)
        .into_iter()
        .filter(|contract| {
            resolve_route(settings, *contract)
                .route
                .is_some_and(|route| AppliedFallback::of(route.fallback).was_downgraded())
        })
        .collect()
}

/// Which doctor runtime key a `local-proc` runtime id reports under.
///
/// The same mapping `ui/assist_settings.rs::readiness` uses, and the reason it
/// is one function: a runtime whose identity is looked up under one key in the
/// dialog and another in the snapshot would show "Ready" beside a blank
/// `runtime_version`.
#[must_use]
pub fn doctor_key(runtime_id: &str) -> Option<&'static str> {
    match runtime_id {
        "whisper.cpp" => Some("whisper"),
        "mms-ctc" | "qwen3-fa" => Some("mms_ctc_aligner"),
        _ => None,
    }
}

/// Resolves one job's whole route graph and freezes it (§5 invariant 3, §6).
#[must_use]
pub fn resolve(
    settings: &AssistSettings,
    kind: WorkflowKind,
    has_lyric_reference: bool,
    facts: &ExecutionFacts,
) -> ExecutionSnapshot {
    let contracts: Vec<ContractSnapshot> = composed_contracts(kind, has_lyric_reference)
        .into_iter()
        .map(|contract| snapshot_contract(settings, contract, facts))
        .collect();
    // §6's credential pair describes **this job**, not the machine. A graph that
    // opens no socket used no credential, so recording one would say a key was
    // involved in producing these artifacts when none was — and the whole reason
    // the field exists is to answer that question for a reader of the manifest.
    // The credential may well be configured; `preflight` reads that separately,
    // from facts, which is where "is one available" belongs.
    let uses_credential = facts.credential_present
        && contracts
            .iter()
            .any(|entry| entry.boundary_applied.rank() > 0);
    ExecutionSnapshot {
        snapshot_schema: SNAPSHOT_SCHEMA.to_string(),
        settings_schema: SETTINGS_SCHEMA.to_string(),
        profile_id: if settings.profile(&settings.active_profile).is_some() {
            settings.active_profile.clone()
        } else {
            RECOMMENDED_PROFILE.to_string()
        },
        resolved_at_utc: facts.resolved_at_utc.clone(),
        contracts,
        catalog_revision: facts.catalog_revision.clone(),
        suitability_revision: Some(suitability::OVERLAY_REVISION.to_string()),
        credential_present: uses_credential,
        credential_fingerprint: uses_credential
            .then(|| facts.credential_fingerprint.clone())
            .flatten(),
    }
}

fn snapshot_contract(
    settings: &AssistSettings,
    contract: ContractId,
    facts: &ExecutionFacts,
) -> ContractSnapshot {
    let resolved = resolve_route(settings, contract);
    let boundary = resolved.boundary();
    let Some(route) = resolved.route.clone() else {
        // An unrouted contract still gets a row. A snapshot that simply omitted
        // it would make "this stage has no route" indistinguishable from "this
        // stage was not part of the job", and the preflight refusal below reads
        // this row to name it.
        return ContractSnapshot {
            contract,
            route_type: RouteType::Builtin,
            runtime_id: "unrouted".to_string(),
            runtime_version: None,
            model_id: String::new(),
            model_sha256: None,
            reasoning_effort: None,
            boundary_applied: Boundary::LocalOnly,
            boundary_confirmed: false,
            audio_scope: None,
            excerpt_spans: Vec::new(),
            provider_constraints: None,
            provider_served: None,
            prompt_version: None,
            prompt_sha256: None,
            schema_version: None,
            fallback_policy: FallbackPolicy::None,
            fallback_taken: false,
            fallback_from: None,
        };
    };
    let key = doctor_key(&route.runtime_id);
    let lookup = |table: &[(String, String)]| -> Option<String> {
        let key = key?;
        table
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };
    let model_id = resolved.model_label();
    let overlay = suitability::row(&model_id, contract);
    ContractSnapshot {
        contract,
        route_type: route.route_type,
        runtime_id: route.runtime_id.clone(),
        runtime_version: lookup(&facts.runtime_versions),
        model_id,
        model_sha256: lookup(&facts.model_digests),
        reasoning_effort: route.reasoning_effort,
        boundary_applied: boundary,
        // §5 invariant 2: a confirmation authorizes *this* job, and only a route
        // that actually leaves the machine can be authorized by one.
        boundary_confirmed: boundary.rank() >= 1 && facts.boundary_confirmed,
        audio_scope: Some(overlay.map_or(
            if boundary.rank() >= 2 {
                AudioScope::WholeTrack
            } else {
                AudioScope::None
            },
            |row| row.audio_scope,
        )),
        excerpt_spans: Vec::new(),
        provider_constraints: (route.route_type == RouteType::OpenRouter).then(|| {
            route
                .provider
                .clone()
                .unwrap_or_else(|| Provider::defaults_for(contract))
        }),
        provider_served: None,
        prompt_version: overlay
            .and_then(|row| row.prompt_version)
            .map(str::to_string),
        prompt_sha256: None,
        schema_version: overlay
            .and_then(|row| row.schema_version)
            .map(str::to_string),
        fallback_policy: AppliedFallback::of(route.fallback).applied,
        fallback_taken: false,
        fallback_from: None,
    }
}

// ---------------------------------------------------------------------------
// Reading the discovery caches
// ---------------------------------------------------------------------------
//
// Both parsers live here rather than beside the file reads for the reason the
// rest of the crate boundary exists: `musializer-runtime` has no serde
// dependency and does not want one, and these are pure byte→value functions
// with edge cases worth a test. `runtime::assist::plan` supplies the bytes.

/// `(revision, model ids)` from an OpenRouter catalog cache document, or `None`
/// when it is absent, unreadable or declares another schema.
///
/// **`None` is "we have not looked", not "there are no models".** The
/// distinction is what stops [`preflight`] refusing a job for a catalog nobody
/// ever fetched.
#[must_use]
pub fn parse_catalog_facts(bytes: &[u8]) -> Option<(String, Vec<String>)> {
    let document: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let schema = document.get("schema_version")?.as_str()?;
    if schema != "musializer.openrouter-catalog/v1" {
        return None;
    }
    let fetched = document
        .get("fetched_at_utc")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let ids = document
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some((format!("{schema}@{fetched}"), ids))
}

/// `(runtime versions, model digests)` from a doctor report, both sorted.
///
/// A runtime the report knows nothing about contributes nothing, so the
/// snapshot records `null` rather than a guessed version.
#[must_use]
#[allow(
    clippy::type_complexity,
    reason = "two sorted association lists, named at every call site"
)]
pub fn parse_doctor_facts(bytes: &[u8]) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return (Vec::new(), Vec::new());
    };
    let Some(runtimes) = document
        .get("runtimes")
        .and_then(serde_json::Value::as_object)
    else {
        return (Vec::new(), Vec::new());
    };
    let mut versions = Vec::new();
    let mut digests = Vec::new();
    for (key, identity) in runtimes {
        if let Some(version) = identity.get("version").and_then(serde_json::Value::as_str) {
            versions.push((key.clone(), version.to_string()));
        }
        if let Some(digest) = identity
            .get("model_sha256")
            .and_then(serde_json::Value::as_str)
        {
            digests.push((key.clone(), digest.to_string()));
        }
    }
    versions.sort();
    digests.sort();
    (versions, digests)
}

// ---------------------------------------------------------------------------
// Preflight (§5 invariant 4)
// ---------------------------------------------------------------------------

/// Why a job may not start. Each variant names one missing or contradictory
/// thing, because "blocked" is one word for four different repairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionBlock {
    /// A composed contract has no route at all.
    Unrouted(ContractId),
    /// The settings schema accepts this future route, but this build has no
    /// dispatcher for it. Running another adapter while recording this one
    /// would make the execution snapshot false.
    UnsupportedRoute {
        contract: ContractId,
        route_type: RouteType,
        runtime_id: String,
    },
    /// A route that needs a model has none.
    NoModel(ContractId),
    /// A remote route with no credential to authorize it.
    NoCredential(ContractId),
    /// Provider constraints that cannot be satisfied, naming which ones.
    NoEndpoint {
        contract: ContractId,
        constraint: String,
    },
}

impl ExecutionBlock {
    /// The short badge form, which is the same vocabulary the settings dialog's
    /// readiness column uses.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unrouted(_) => "No route chosen",
            Self::UnsupportedRoute { .. } => "Route not implemented",
            Self::NoModel(_) => NO_MODEL,
            Self::NoCredential(_) => NO_KEY,
            Self::NoEndpoint { .. } => NO_ENDPOINT,
        }
    }

    #[must_use]
    pub fn contract(&self) -> ContractId {
        match self {
            Self::Unrouted(contract)
            | Self::NoModel(contract)
            | Self::NoCredential(contract)
            | Self::NoEndpoint { contract, .. } => *contract,
            Self::UnsupportedRoute { contract, .. } => *contract,
        }
    }

    /// The sentence the user is shown. Every one of them names the repair, and
    /// the `NoEndpoint` arm names the constraint that emptied the set rather
    /// than reporting that "something" is wrong.
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            Self::Unrouted(contract) => format!(
                "{} ({}) has no route. Choose one in AI settings \u{2192} Routing.",
                contract.human_label(),
                contract.token(),
            ),
            Self::UnsupportedRoute {
                contract,
                route_type,
                runtime_id,
            } => format!(
                "{} ({}) is configured as {}/{}, but this build has no executor for that route. \
                 Choose the implemented route in AI settings \u{2192} Routing.",
                contract.human_label(),
                contract.token(),
                route_type.token(),
                runtime_id,
            ),
            Self::NoModel(contract) => format!(
                "{} ({}) has no model chosen. Pick one in AI settings \u{2192} Routing.",
                contract.human_label(),
                contract.token(),
            ),
            Self::NoCredential(contract) => format!(
                "{} ({}) is routed to OpenRouter and no key is configured. Add one in AI \
                 settings \u{2192} OpenRouter, or route this task locally.",
                contract.human_label(),
                contract.token(),
            ),
            Self::NoEndpoint {
                contract,
                constraint,
            } => format!(
                "{} ({}) has no eligible endpoint: {constraint}. Relax the constraint in AI \
                 settings, or route this task locally. It is never weakened silently.",
                contract.human_label(),
                contract.token(),
            ),
        }
    }
}

/// Everything the pre-spawn check needs that the snapshot does not carry.
#[derive(Clone, Debug, Default)]
pub struct PreflightFacts {
    pub credential_present: bool,
    /// The model ids the last catalog fetch reported, or `None` when the
    /// catalog was never fetched. `None` is "we have not looked", which is not
    /// grounds to refuse a job.
    pub catalog_model_ids: Option<Vec<String>>,
}

/// Everything that must be true before a process is spawned (§5 invariant 4).
///
/// ## What this can and cannot decide offline
///
/// A credential's absence, a missing model and a self-contradictory provider
/// selection are all decidable here, and all three block. Whether a
/// zero-data-retention endpoint exists **for a given model** is not: the
/// normalized catalog (`tools/provider_catalog.py`) carries modalities, context
/// and price and no endpoint list at all, so claiming "no ZDR endpoint" from it
/// would be an invented fact.
///
/// So the ZDR rule is enforced in two places rather than pretended in one.
/// Here, a `zdr_required` route whose provider allow-list is emptied by
/// `ignore`, by a disjoint `order` with fallbacks off, or by a zero price bound
/// is refused **before** anything spawns and the message names ZDR alongside
/// the constraint that emptied it. Beyond that, `provider.zdr` is sent on the
/// request and OpenRouter refuses rather than substituting, and the helper
/// surfaces that refusal verbatim. Neither path ever weakens the constraint.
#[must_use]
pub fn preflight(snapshot: &ExecutionSnapshot, facts: &PreflightFacts) -> Vec<ExecutionBlock> {
    let mut blocks = Vec::new();
    for entry in &snapshot.contracts {
        if entry.runtime_id == "unrouted" {
            blocks.push(ExecutionBlock::Unrouted(entry.contract));
            continue;
        }
        if !entry.contract.route_is_implemented(
            entry.route_type,
            &entry.runtime_id,
            (!entry.model_id.is_empty()).then_some(entry.model_id.as_str()),
        ) {
            blocks.push(ExecutionBlock::UnsupportedRoute {
                contract: entry.contract,
                route_type: entry.route_type,
                runtime_id: entry.runtime_id.clone(),
            });
            continue;
        }
        let needs_model = !matches!(entry.route_type, RouteType::Builtin | RouteType::Codex);
        if needs_model && entry.model_id.is_empty() {
            blocks.push(ExecutionBlock::NoModel(entry.contract));
            continue;
        }
        if entry.route_type != RouteType::OpenRouter {
            continue;
        }
        if !facts.credential_present {
            blocks.push(ExecutionBlock::NoCredential(entry.contract));
            continue;
        }
        if let Some(provider) = &entry.provider_constraints {
            if let Some(constraint) = unsatisfiable_constraint(provider) {
                blocks.push(ExecutionBlock::NoEndpoint {
                    contract: entry.contract,
                    constraint,
                });
                continue;
            }
        }
        if let Some(catalog) = &facts.catalog_model_ids {
            if !catalog.iter().any(|id| id == &entry.model_id) {
                blocks.push(ExecutionBlock::NoEndpoint {
                    contract: entry.contract,
                    constraint: format!("the last catalog fetch does not list {}", entry.model_id),
                });
            }
        }
    }
    blocks
}

/// Names the constraint that leaves a provider selection with nothing in it, or
/// `None` when the selection is satisfiable as far as this side can tell.
///
/// Every arm is a **closed** set being emptied — an `only` list, or an `order`
/// list with fallbacks off. An open selection (no `only`, fallbacks on) can
/// always still resolve to some endpoint, and refusing one would be this
/// function inventing the fact it does not have.
#[must_use]
pub fn unsatisfiable_constraint(provider: &Provider) -> Option<String> {
    let zdr = if provider.zdr_required {
        " with zero data retention required"
    } else {
        ""
    };
    if !provider.only.is_empty() {
        let survivors: Vec<String> = provider
            .only
            .iter()
            .filter(|slug| !provider.ignore.contains(slug))
            .cloned()
            .collect();
        if survivors.is_empty() {
            return Some(format!(
                "provider.ignore removes every entry of provider.only{zdr}"
            ));
        }
        if !provider.allow_fallbacks
            && !provider.order.is_empty()
            && !provider.order.iter().any(|slug| survivors.contains(slug))
        {
            return Some(format!(
                "provider.order names no provider that survives provider.only and \
                 provider.ignore, and fallbacks are off{zdr}"
            ));
        }
    } else if !provider.allow_fallbacks
        && !provider.order.is_empty()
        && provider
            .order
            .iter()
            .all(|slug| provider.ignore.contains(slug))
    {
        return Some(format!(
            "provider.ignore removes every entry of provider.order and fallbacks are off{zdr}"
        ));
    }
    for (name, bound) in [
        ("max_price_prompt", provider.max_price_prompt),
        ("max_price_completion", provider.max_price_completion),
        ("max_price_audio", provider.max_price_audio),
    ] {
        if bound == Some(0.0) {
            return Some(format!("{name} is 0, which admits no paid endpoint{zdr}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::settings::Profile;
    use std::collections::BTreeMap;

    fn facts() -> ExecutionFacts {
        ExecutionFacts {
            resolved_at_utc: "2026-08-05T12:00:00Z".to_string(),
            credential_present: true,
            credential_fingerprint: Some("0a1b2c3d".to_string()),
            catalog_revision: Some(
                "musializer.openrouter-catalog/v1@2026-08-05T10:00:00Z".to_string(),
            ),
            runtime_versions: vec![("whisper".to_string(), "whisper.cpp 1.8.6".to_string())],
            model_digests: vec![("whisper".to_string(), "a".repeat(64))],
            boundary_confirmed: true,
        }
    }

    /// The same vectors `ui/assist_settings.rs`'s parser test pins, read the
    /// other way. Two independent implementations of the same civil-date
    /// arithmetic that disagree would put one instant in the badge and another
    /// in the snapshot.
    #[test]
    fn the_timestamp_formatter_inverts_the_dialogs_parser() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_785_888_000), "2026-08-05T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_785_931_200), "2026-08-05T12:00:00Z");
        assert_eq!(
            format_rfc3339_utc(1_785_888_000 + 45_296),
            "2026-08-05T12:34:56Z"
        );
        // A leap day, and the last second of a year, where an off-by-one in the
        // era arithmetic would show.
        assert_eq!(format_rfc3339_utc(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(format_rfc3339_utc(1_767_225_599), "2025-12-31T23:59:59Z");
    }

    #[test]
    fn the_recommended_graph_for_a_lyrics_job_is_entirely_local() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            true,
            &facts(),
        );
        assert!(!snapshot.has_remote_route());
        assert!(!snapshot.sends_audio_off_machine());
        let tokens: Vec<&str> = snapshot
            .contracts
            .iter()
            .map(|entry| entry.contract.token())
            .collect();
        assert_eq!(
            tokens,
            vec!["TC-MEASURED", "TC-COARSE", "TC-ALIGN", "TC-PLAN"]
        );
        for entry in &snapshot.contracts {
            assert_eq!(entry.boundary_applied, Boundary::LocalOnly);
            assert!(!entry.boundary_confirmed);
            assert!(entry.provider_constraints.is_none());
        }
        assert!(preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: false,
                catalog_model_ids: None,
            }
        )
        .is_empty());
    }

    #[test]
    fn an_auto_discovered_local_model_names_its_runtime_instead_of_not_chosen() {
        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.model_id = None;
        let resolved = ResolvedRoute {
            contract: ContractId::Coarse,
            route: Some(route),
            origin: RouteOrigin::Override,
        };
        assert_eq!(resolved.model_label(), "whisper.cpp");
    }

    #[test]
    fn a_job_without_authored_lyrics_composes_the_wording_review() {
        let with = composed_contracts(WorkflowKind::Lyrics, true);
        let without = composed_contracts(WorkflowKind::Lyrics, false);
        assert!(!with.contains(&ContractId::Wording));
        assert!(without.contains(&ContractId::Wording));
        // And the wording review sits between the evidence pass and the
        // acoustic stage, which is the order `run_assist` runs them in.
        let index = |list: &[ContractId], id: ContractId| {
            list.iter().position(|entry| *entry == id).unwrap()
        };
        assert!(index(&without, ContractId::Coarse) < index(&without, ContractId::Wording));
        assert!(index(&without, ContractId::Wording) < index(&without, ContractId::Align));
    }

    #[test]
    fn no_workflow_composes_the_unbenchmarked_verify_contract() {
        for kind in [
            WorkflowKind::Lyrics,
            WorkflowKind::Sections,
            WorkflowKind::Mimo,
            WorkflowKind::All,
        ] {
            for reference in [true, false] {
                assert!(!composed_contracts(kind, reference).contains(&ContractId::Verify));
                // And every workflow measures and plans.
                let composed = composed_contracts(kind, reference);
                assert_eq!(composed.first(), Some(&ContractId::Measured));
                assert_eq!(composed.last(), Some(&ContractId::Plan));
            }
        }
    }

    #[test]
    fn a_mimo_job_records_audio_leaving_and_the_zdr_constraint_as_sent() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let semantic = snapshot.contract(ContractId::Semantic).expect("composed");
        assert_eq!(semantic.boundary_applied, Boundary::AudioLeavesMachine);
        assert!(semantic.boundary_confirmed);
        assert_eq!(semantic.model_id, "xiaomi/mimo-v2.5");
        let provider = semantic.provider_constraints.as_ref().expect("constraints");
        assert!(provider.zdr_required, "an audio contract defaults to ZDR");
        assert!(!provider.allow_fallbacks);
        assert!(snapshot.has_remote_route());
        assert!(snapshot.sends_audio_off_machine());
    }

    /// The gate on the credential hand-off, as a truth table rather than a
    /// comment: a local graph never authorizes one, and a remote graph does so
    /// only with this job's own confirmation.
    #[test]
    fn only_a_confirmed_remote_graph_authorizes_a_credential() {
        let settings = AssistSettings::default();
        let confirmed = facts();
        let unconfirmed = ExecutionFacts {
            boundary_confirmed: false,
            ..facts()
        };
        for (kind, reference) in [
            (WorkflowKind::Lyrics, true),
            (WorkflowKind::Sections, true),
            (WorkflowKind::Lyrics, false),
        ] {
            let snapshot = resolve(&settings, kind, reference, &confirmed);
            // A `TC-WORDING` Codex route is remote, so the lyrics-without-a-sheet
            // case is deliberately in this list: what makes it not need a key is
            // that Codex authenticates itself, which `preflight` decides, not
            // this function.
            assert_eq!(
                snapshot.authorizes_credential(),
                snapshot.has_remote_route(),
                "{kind:?} reference={reference}"
            );
        }
        assert!(!resolve(&settings, WorkflowKind::Lyrics, true, &confirmed).authorizes_credential());
        assert!(resolve(&settings, WorkflowKind::Mimo, true, &confirmed).authorizes_credential());
        assert!(!resolve(&settings, WorkflowKind::Mimo, true, &unconfirmed).authorizes_credential());
    }

    #[test]
    fn an_unconfirmed_boundary_is_never_recorded_as_confirmed() {
        let unconfirmed = ExecutionFacts {
            boundary_confirmed: false,
            ..facts()
        };
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &unconfirmed,
        );
        assert!(
            !snapshot
                .contract(ContractId::Semantic)
                .unwrap()
                .boundary_confirmed
        );
    }

    /// §5 invariant 1, as a measurement over the whole applied graph rather than
    /// a sentence in a comment.
    #[test]
    fn no_applied_fallback_in_any_resolvable_graph_can_raise_a_boundary() {
        for policy in [
            FallbackPolicy::None,
            FallbackPolicy::Ask,
            FallbackPolicy::LocalOnly,
            FallbackPolicy::SameBoundary,
        ] {
            let applied = AppliedFallback::of(policy);
            assert_ne!(applied.applied, FallbackPolicy::Ask);
            for failed in [
                Boundary::LocalOnly,
                Boundary::TextLeavesMachine,
                Boundary::AudioLeavesMachine,
            ] {
                for candidate in [
                    Boundary::LocalOnly,
                    Boundary::TextLeavesMachine,
                    Boundary::AudioLeavesMachine,
                ] {
                    if applied
                        .applied
                        .permits_automatic_substitute(failed, candidate)
                    {
                        assert!(candidate.rank() <= failed.rank());
                    }
                }
            }
        }
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::All,
            false,
            &facts(),
        );
        assert!(!any_fallback_can_raise_boundary(&snapshot.contracts));
    }

    #[test]
    fn ask_is_applied_as_none_and_says_so() {
        let applied = AppliedFallback::of(FallbackPolicy::Ask);
        assert_eq!(applied.stored, FallbackPolicy::Ask);
        assert_eq!(applied.applied, FallbackPolicy::None);
        assert!(applied.was_downgraded());
        assert!(!AppliedFallback::of(FallbackPolicy::LocalOnly).was_downgraded());

        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        route.fallback = FallbackPolicy::Ask;
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Semantic, route)]),
        });
        settings.active_profile = "studio".to_string();
        settings.validate().expect("ask is a legal stored policy");
        let snapshot = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        assert_eq!(
            snapshot
                .contract(ContractId::Semantic)
                .unwrap()
                .fallback_policy,
            FallbackPolicy::None,
        );
    }

    #[test]
    fn a_missing_credential_blocks_a_remote_route_and_names_the_repair() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: false,
                catalog_model_ids: None,
            },
        );
        assert_eq!(
            blocks,
            vec![ExecutionBlock::NoCredential(ContractId::Semantic)]
        );
        assert_eq!(blocks[0].label(), NO_KEY);
        let sentence = blocks[0].sentence();
        assert!(sentence.contains("TC-SEMANTIC"), "{sentence}");
        assert!(sentence.contains("AI settings"), "{sentence}");
        assert!(sentence.contains("route this task locally"), "{sentence}");
    }

    #[test]
    fn a_schema_legal_route_without_an_executor_is_refused_before_spawn() {
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.route_type = RouteType::OpenRouter;
        route.runtime_id = "openrouter".to_string();
        route.model_id = Some("google/gemini-test".to_string());
        route.provider = Some(Provider::defaults_for(ContractId::Coarse));
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Coarse, route)]),
        });
        settings.active_profile = "studio".to_string();
        settings
            .validate()
            .expect("future route remains schema-legal");
        let snapshot = resolve(&settings, WorkflowKind::Lyrics, true, &facts());
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: true,
                catalog_model_ids: None,
            },
        );
        assert_eq!(
            blocks,
            vec![ExecutionBlock::UnsupportedRoute {
                contract: ContractId::Coarse,
                route_type: RouteType::OpenRouter,
                runtime_id: "openrouter".to_string(),
            }]
        );
        assert!(blocks[0].sentence().contains("no executor"));
    }

    #[test]
    fn contradictory_provider_constraints_block_and_name_zdr() {
        let mut provider = Provider::defaults_for(ContractId::Semantic);
        provider.only = vec!["fireworks".to_string()];
        provider.ignore = vec!["fireworks".to_string()];
        assert!(provider.zdr_required);
        let constraint = unsatisfiable_constraint(&provider).expect("no endpoint survives");
        assert!(constraint.contains("provider.ignore"), "{constraint}");
        assert!(
            constraint.contains("zero data retention"),
            "the message must name the constraint the user set: {constraint}"
        );

        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        route.provider = Some(provider);
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Semantic, route)]),
        });
        settings.active_profile = "studio".to_string();
        let snapshot = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: true,
                catalog_model_ids: None,
            },
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].label(), NO_ENDPOINT);
        assert!(blocks[0].sentence().contains("zero data retention"));
    }

    #[test]
    fn an_open_provider_selection_is_never_refused_offline() {
        // The direction that matters: this side must not invent "no endpoint"
        // for constraints it cannot evaluate. Only a *closed* set being emptied
        // is grounds to refuse.
        let plain = Provider::defaults_for(ContractId::Semantic);
        assert_eq!(unsatisfiable_constraint(&plain), None);
        let ordered = Provider {
            order: vec!["fireworks".to_string()],
            allow_fallbacks: true,
            ..Provider::defaults_for(ContractId::Semantic)
        };
        assert_eq!(unsatisfiable_constraint(&ordered), None);
        let ignoring = Provider {
            ignore: vec!["fireworks".to_string()],
            ..Provider::defaults_for(ContractId::Semantic)
        };
        assert_eq!(unsatisfiable_constraint(&ignoring), None);
        let priced = Provider {
            max_price_audio: Some(12.5),
            ..Provider::defaults_for(ContractId::Semantic)
        };
        assert_eq!(unsatisfiable_constraint(&priced), None);
    }

    #[test]
    fn the_catalog_parser_separates_absent_from_empty() {
        let (revision, ids) = parse_catalog_facts(
            br#"{"schema_version":"musializer.openrouter-catalog/v1",
                 "fetched_at_utc":"2026-08-05T10:00:00Z",
                 "models":[{"id":"xiaomi/mimo-v2.5"},{"id":"openai/gpt-4o"},{"name":"no id"}]}"#,
        )
        .expect("a well-formed catalog");
        assert_eq!(
            revision,
            "musializer.openrouter-catalog/v1@2026-08-05T10:00:00Z"
        );
        assert_eq!(ids, vec!["xiaomi/mimo-v2.5", "openai/gpt-4o"]);
        // A fetched catalog with no models is a real answer and parses.
        let (_, empty) = parse_catalog_facts(
            br#"{"schema_version":"musializer.openrouter-catalog/v1","models":[]}"#,
        )
        .expect("an empty catalog is still a catalog");
        assert!(empty.is_empty());
        // Another schema, or nothing at all, is "we have not looked".
        assert_eq!(
            parse_catalog_facts(br#"{"schema_version":"musializer.openrouter-catalog/v2"}"#),
            None
        );
        assert_eq!(parse_catalog_facts(b"{ broken"), None);
    }

    #[test]
    fn the_doctor_parser_reports_only_what_the_report_measured() {
        let (versions, digests) = parse_doctor_facts(
            br#"{"schema_version":"musializer.doctor/v1","runtimes":{
                "whisper":{"state":"available","version":"whisper.cpp 1.8.6","model_sha256":"beef"},
                "mms_ctc_aligner":{"state":"missing"}}}"#,
        );
        assert_eq!(
            versions,
            vec![("whisper".to_string(), "whisper.cpp 1.8.6".to_string())]
        );
        assert_eq!(digests, vec![("whisper".to_string(), "beef".to_string())]);
        assert_eq!(parse_doctor_facts(b"{ broken"), (Vec::new(), Vec::new()));
        assert_eq!(parse_doctor_facts(b"{}"), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_never_fetched_catalog_does_not_refuse_a_job() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        assert!(preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: true,
                catalog_model_ids: None,
            }
        )
        .is_empty());
        // But a catalog that *was* fetched and does not list the model does.
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                credential_present: true,
                catalog_model_ids: Some(vec!["openai/gpt-4o-audio-preview".to_string()]),
            },
        );
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].sentence().contains("xiaomi/mimo-v2.5"));
    }

    #[test]
    fn the_snapshot_round_trips_and_writes_the_same_bytes_twice() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::All,
            false,
            &facts(),
        );
        let bytes = snapshot.to_bytes().unwrap();
        assert_eq!(ExecutionSnapshot::parse(&bytes).unwrap(), snapshot);
        assert_eq!(snapshot.to_bytes().unwrap(), bytes);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"snapshot_schema\": \"musializer.assist-execution/v1\""));
        // Every §6 field is present, including the absent ones as null: the
        // snapshot is provenance, so "not written" and "no value" must differ.
        for field in [
            "settings_schema",
            "profile_id",
            "resolved_at_utc",
            "contracts",
            "catalog_revision",
            "suitability_revision",
            "credential_present",
            "credential_fingerprint",
            "runtime_version",
            "model_sha256",
            "reasoning_effort",
            "boundary_applied",
            "boundary_confirmed",
            "audio_scope",
            "excerpt_spans",
            "provider_constraints",
            "provider_served",
            "prompt_version",
            "prompt_sha256",
            "schema_version",
            "fallback_policy",
            "fallback_taken",
            "fallback_from",
        ] {
            assert!(text.contains(&format!("\"{field}\"")), "missing {field}");
        }
        assert!(
            !text.contains("0a1b2c3d0"),
            "the fingerprint is 8 hex, no more"
        );
        assert!(text.contains("\"credential_fingerprint\": \"0a1b2c3d\""));
    }

    /// §6's credential pair describes the job. A graph that opens no socket used
    /// no credential, and saying otherwise would put a key's fingerprint in the
    /// provenance of artifacts it had nothing to do with.
    #[test]
    fn a_local_only_job_records_no_credential_even_when_one_is_configured() {
        assert!(facts().credential_present, "one really is configured");
        let local = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            true,
            &facts(),
        );
        assert!(!local.credential_present);
        assert_eq!(local.credential_fingerprint, None);

        let remote = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        assert!(remote.credential_present);
        assert_eq!(remote.credential_fingerprint.as_deref(), Some("0a1b2c3d"));
    }

    /// §5 rule 7 is enforced by `tools/external_analysis.py`, which compares the
    /// route-identity subset of a contract row against a cached artifact's
    /// `provenance.execution.route_identity`. There is deliberately **one**
    /// implementation of that subset, and it is the helper's: a second one here
    /// would be an unused definition that a later reader could mistake for the
    /// authoritative one, and the two could drift with nothing to catch it.
    ///
    /// What this side owes is that the row really does change when a user
    /// changes a route, which is what that comparison depends on.
    #[test]
    fn a_route_change_changes_the_contract_row_the_helper_compares() {
        let base = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let original = base.contract(ContractId::Semantic).unwrap().clone();

        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        route.model_id = Some("google/gemini-2.5-flash".to_string());
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Semantic, route.clone())]),
        });
        settings.active_profile = "studio".to_string();
        let changed_model = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        assert_ne!(
            changed_model
                .contract(ContractId::Semantic)
                .unwrap()
                .model_id,
            original.model_id
        );

        // And a constraint change with the same model is also a different route.
        route.model_id = Some("xiaomi/mimo-v2.5".to_string());
        let mut provider = Provider::defaults_for(ContractId::Semantic);
        provider.zdr_required = false;
        route.provider = Some(provider);
        settings.profiles[0]
            .routes
            .insert(ContractId::Semantic, route);
        let changed_zdr = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        let rerouted = changed_zdr.contract(ContractId::Semantic).unwrap();
        assert_eq!(rerouted.model_id, original.model_id);
        assert_ne!(rerouted.provider_constraints, original.provider_constraints);
    }

    #[test]
    fn a_stored_override_is_what_the_snapshot_records() {
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.model_id = Some("whisper.cpp".to_string());
        route.runtime_id = "whisper.cpp".to_string();
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::new(),
        });
        settings.active_profile = "studio".to_string();
        let snapshot = resolve(&settings, WorkflowKind::Lyrics, true, &facts());
        assert_eq!(snapshot.profile_id, "studio");
        let coarse = snapshot.contract(ContractId::Coarse).unwrap();
        assert_eq!(coarse.runtime_version.as_deref(), Some("whisper.cpp 1.8.6"));
        assert_eq!(
            coarse.model_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
        // An unresolvable active profile still records `recommended` rather than
        // a name nothing inherits from.
        let unknown = AssistSettings {
            active_profile: RECOMMENDED_PROFILE.to_string(),
            ..AssistSettings::default()
        };
        assert_eq!(
            resolve(&unknown, WorkflowKind::Lyrics, true, &facts()).profile_id,
            RECOMMENDED_PROFILE
        );
    }

    #[test]
    fn no_snapshot_field_can_hold_a_credential() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::All,
            false,
            &facts(),
        );
        let text = String::from_utf8(snapshot.to_bytes().unwrap()).unwrap();
        assert!(!text.contains("sk-or"));
        assert!(!text.contains("api_key"));
        // The one credential fact it carries is a presence flag and a
        // fingerprint (E4).
        let planted = r#"{"snapshot_schema":"musializer.assist-execution/v1","settings_schema":"x",
            "profile_id":"p","resolved_at_utc":"t","contracts":[],"catalog_revision":null,
            "suitability_revision":null,"credential_present":true,"credential_fingerprint":null,
            "api_key":"sk-or-v1-MUSICANARY7Q4X2ZK9"}"#;
        assert!(ExecutionSnapshot::parse(planted.as_bytes()).is_err());
    }
}
