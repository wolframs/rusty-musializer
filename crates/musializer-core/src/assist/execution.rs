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
/// The fourth: the model is still there and can no longer do the job (§5
/// invariant 5).
pub const MODALITY_LOST: &str = "Model lost a required modality";
/// The credentials file exists and was refused rather than repaired (§3). A
/// separate sentence from [`NO_KEY`], because "add a key" is the wrong
/// instruction for a user who has one (audit A3).
pub const KEY_REFUSED: &str = "Key file refused";
/// Codex discovery looked everywhere it knows and found nothing.
pub const CODEX_NOT_FOUND: &str = "codex not found";
/// `local_runtimes.codex_bin` names something that is not a runnable file.
/// `settings.rs`'s rule is that a set-but-missing path is a loud failure and
/// never a silent fallback, which is what this block is (audit B6).
pub const CODEX_OVERRIDE_MISSING: &str = "codex_bin path is wrong";
/// A doctor report measured this local runtime and did not find it usable.
pub const RUNTIME_UNAVAILABLE: &str = "Runtime not available";

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

    /// The model identity a snapshot **records**, which is not always the one
    /// the picker displays.
    ///
    /// The difference is one case and it is audit A2: a remote route with no
    /// model chosen displays as `not chosen` — a phrase, for a column that has
    /// to say something — and records as the empty string, because a snapshot
    /// field named `model_id` holding a sentence is a job asking a provider for
    /// a model literally called "not chosen". Empty is what makes
    /// [`ExecutionBlock::NoModel`] reachable, which is the block that names the
    /// actual repair.
    #[must_use]
    pub fn model_id_recorded(&self) -> String {
        let Some(route) = &self.route else {
            return String::new();
        };
        match (&route.model_id, route.route_type) {
            (Some(id), _) => id.clone(),
            (None, RouteType::Codex) => CODEX_DEFAULT_LABEL.to_string(),
            (None, RouteType::Builtin | RouteType::LocalProc) => route.runtime_id.clone(),
            (None, _) => String::new(),
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

    /// Whether any route in this graph asks for zero data retention.
    ///
    /// The snapshot is the authority on what is sent, and §6 defines
    /// `provider_constraints` as "as sent" — so the request has to be composed
    /// from this rather than from the workflow's name. Audit A4: `--zdr` was
    /// passed unconditionally for every model mode, which made the Privacy
    /// pane's ZDR toggle a control that could be pressed and could not turn
    /// anything off, and made the frozen record understate the constraint the
    /// job actually ran under. The helper still ORs this with the route's own
    /// requirement (`external_analysis.py:1872`), so a contract whose
    /// constraint says ZDR keeps it whatever this returns; the toggle now
    /// governs exactly the routes where it is genuinely optional.
    #[must_use]
    pub fn requires_zdr(&self) -> bool {
        self.contracts.iter().any(|entry| {
            entry
                .provider_constraints
                .as_ref()
                .is_some_and(|provider| provider.zdr_required)
        })
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
    let model_id = resolved.model_id_recorded();
    let overlay = suitability::row(&model_id, contract);
    // §6's `audio_scope` is a statement about **this job**, so it cannot be
    // wider than the boundary this row applied. The overlay's value is a
    // property of the (model, contract) pair in the abstract — mms-ctc and
    // whisper.cpp both carry `whole-track` there — and reading it straight
    // wrote `boundary_applied: local-only` beside `audio_scope: whole-track`
    // on every default Timed lyrics job, on lanes that open no socket (audit
    // A5). The boundary sets the ceiling; the overlay may only narrow it.
    let scope_ceiling = if boundary.rank() >= 2 {
        AudioScope::WholeTrack
    } else {
        AudioScope::None
    };
    let audio_scope = match overlay.map(|row| row.audio_scope) {
        Some(scope) if scope.rank() < scope_ceiling.rank() => scope,
        _ => scope_ceiling,
    };
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
        audio_scope: Some(audio_scope),
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

/// One model row of the normalized catalog cache, as much of it as preflight
/// needs.
///
/// The modalities ride along with the id because membership alone cannot answer
/// §5 invariant 5: a refreshed catalog that still lists a model but no longer
/// says it takes audio has invalidated the route just as surely as dropping it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogModelFacts {
    pub id: String,
    /// The modalities the catalog reported. **Empty means "not reported"**, not
    /// "none" — `tools/provider_catalog.py` normalizes a missing
    /// `architecture` block to an empty list, and refusing a job over an
    /// unreported field would be inventing the fact this side does not have.
    pub input_modalities: Vec<String>,
}

impl CatalogModelFacts {
    /// Whether this row contradicts a required modality. An unreported list
    /// never contradicts anything (see [`Self::input_modalities`]).
    #[must_use]
    pub fn refuses_input(&self, modality: &str) -> bool {
        !self.input_modalities.is_empty()
            && !self.input_modalities.iter().any(|entry| entry == modality)
    }
}

/// `(revision, models)` from an OpenRouter catalog cache document, or `None`
/// when it is absent, unreadable or declares another schema.
///
/// **`None` is "we have not looked", not "there are no models".** The
/// distinction is what stops [`preflight`] refusing a job for a catalog nobody
/// ever fetched.
#[must_use]
pub fn parse_catalog_facts(bytes: &[u8]) -> Option<(String, Vec<CatalogModelFacts>)> {
    let document: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let schema = document.get("schema_version")?.as_str()?;
    if schema != "musializer.openrouter-catalog/v1" {
        return None;
    }
    let fetched = document
        .get("fetched_at_utc")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let models = document
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    let id = model.get("id").and_then(serde_json::Value::as_str)?;
                    Some(CatalogModelFacts {
                        id: id.to_string(),
                        input_modalities: model
                            .get("input_modalities")
                            .and_then(serde_json::Value::as_array)
                            .map(|list| {
                                list.iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some((format!("{schema}@{fetched}"), models))
}

/// The one doctor-report schema this build reads (`tools/musializer_doctor.py`).
pub const DOCTOR_SCHEMA: &str = "musializer.doctor/v1";

/// Everything a `musializer.doctor/v1` report contributes, read once.
///
/// The provenance half and the readiness half used to be two public parsers
/// over the same bytes — that one fills the snapshot's `runtime_versions` and
/// `model_digests`, this one decides whether a job may start (audit A1) — which
/// meant two independent schema policies over one document. They are fields of
/// one reading now, so the version is checked in one place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctorFacts {
    /// Sorted `(runtime key, version)`. A runtime the report knows nothing
    /// about contributes nothing, so the snapshot records `null` rather than a
    /// guessed version.
    pub runtime_versions: Vec<(String, String)>,
    /// Sorted `(runtime key, model sha256)`.
    pub model_digests: Vec<(String, String)>,
    /// Sorted `(runtime key, state)`. A runtime the report does not mention is
    /// absent from the list, which [`PreflightFacts::runtime`] reads as
    /// `Unmeasured` rather than as a refusal.
    pub runtimes: Vec<(String, RuntimeFact)>,
}

/// What a doctor report yielded, or why it yielded nothing.
///
/// Audit B5: both readers used to take *any* JSON object, with every field
/// `#[serde(default)]` and the report's own `schema_version` never looked at —
/// so a renamed `runtimes` key, or a future `musializer.doctor/v2` whose
/// identities are shaped differently, produced a cheerful "Doctor finished;
/// runtime identities updated." over an empty list and a snapshot with no
/// runtime provenance at all. `parse_catalog_facts`, ten lines up in the same
/// file, checked its schema: one file, two policies. This is the other one.
///
/// A mismatch is deliberately **not** a refusal to start a job: an unmeasured
/// runtime has never been grounds to block (`PreflightFacts::runtime`), and a
/// foreign report is exactly as unmeasured as no report. It has to be *visible*,
/// which is what the variants are for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DoctorReading {
    /// No report was named. The ordinary case: nothing runs the doctor for a
    /// job.
    NotTaken,
    /// The bytes are not JSON, or not a JSON object.
    Unreadable(String),
    /// A JSON object declaring a schema this build does not read. Carries what
    /// it declared, because "which version" is the whole repair.
    ForeignSchema(String),
    Report(DoctorFacts),
}

impl DoctorReading {
    /// The facts, or the empty set for every non-report state.
    #[must_use]
    pub fn facts(&self) -> DoctorFacts {
        match self {
            DoctorReading::Report(facts) => facts.clone(),
            _ => DoctorFacts::default(),
        }
    }

    /// One greppable token, so a report line can say which of the four
    /// happened. Three of them show the same empty runtime list.
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            DoctorReading::NotTaken => "not-taken".to_string(),
            DoctorReading::Unreadable(reason) => format!("unreadable({reason})"),
            DoctorReading::ForeignSchema(schema) => format!("foreign({schema})"),
            DoctorReading::Report(facts) => format!("v1({})", facts.runtimes.len()),
        }
    }
}

/// Reads a doctor report, checking the schema it declares.
#[must_use]
pub fn parse_doctor_reading(bytes: &[u8]) -> DoctorReading {
    let Ok(document) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return DoctorReading::Unreadable("not JSON".to_string());
    };
    let Some(object) = document.as_object() else {
        return DoctorReading::Unreadable("not a JSON object".to_string());
    };
    match object
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
    {
        Some(DOCTOR_SCHEMA) => {}
        Some(other) => return DoctorReading::ForeignSchema(other.to_string()),
        None => return DoctorReading::ForeignSchema("none declared".to_string()),
    }
    let Some(runtimes) = object
        .get("runtimes")
        .and_then(serde_json::Value::as_object)
    else {
        return DoctorReading::Report(DoctorFacts::default());
    };
    let mut facts = DoctorFacts::default();
    for (key, identity) in runtimes {
        if let Some(version) = identity.get("version").and_then(serde_json::Value::as_str) {
            facts
                .runtime_versions
                .push((key.clone(), version.to_string()));
        }
        if let Some(digest) = identity
            .get("model_sha256")
            .and_then(serde_json::Value::as_str)
        {
            facts.model_digests.push((key.clone(), digest.to_string()));
        }
        if let Some(state) = identity.get("state").and_then(serde_json::Value::as_str) {
            facts.runtimes.push((
                key.clone(),
                if runtime_state_is_available(state) {
                    RuntimeFact::Available
                } else {
                    RuntimeFact::Unavailable {
                        remediation: identity
                            .get("remediation")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(state)
                            .to_string(),
                    }
                },
            ));
        }
    }
    facts.runtime_versions.sort();
    facts.model_digests.sort();
    facts.runtimes.sort_by(|left, right| left.0.cmp(&right.0));
    DoctorReading::Report(facts)
}

/// The two doctor states that mean "usable". Named here so the dialog's badge
/// and the pre-spawn gate cannot spell the vocabulary differently.
#[must_use]
pub fn runtime_state_is_available(state: &str) -> bool {
    matches!(state, "ok" | "available")
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
    /// A remote route whose credentials file exists and was **refused** — a
    /// mode other users can read, or bytes that are not this store (§3, "read
    /// refusal"). Distinct from [`Self::NoCredential`] because the repair is
    /// the opposite one: the key is there, and telling the user to add one is
    /// a dead end (audit A3).
    CredentialRefused {
        contract: ContractId,
        path: String,
        /// The mode as it stands, so the `chmod` is a command rather than a
        /// puzzle. `None` when the file was refused for a reason other than
        /// its permissions.
        mode: Option<u32>,
    },
    /// A Codex route on a machine where discovery found no `codex`.
    CodexNotFound(ContractId),
    /// A Codex route whose `local_runtimes.codex_bin` is set and is not a
    /// runnable file. `settings.rs:196-201` states the rule this enforces: a
    /// set-but-missing path is a loud failure, never a silent fallback onto
    /// whatever `codex` happens to be on `PATH` (audit B6).
    CodexOverrideMissing {
        contract: ContractId,
        /// The configured path, named because "wrong path" without the path is
        /// not a repair.
        path: String,
        reason: String,
    },
    /// A local runtime a doctor report measured and did not find usable.
    RuntimeUnavailable {
        contract: ContractId,
        runtime_id: String,
        /// The doctor's own remediation where it gave one, else its state.
        remediation: String,
    },
    /// Provider constraints that cannot be satisfied, naming which ones.
    NoEndpoint {
        contract: ContractId,
        constraint: String,
    },
    /// The model is still in the catalog, but the catalog no longer reports the
    /// modality this contract needs (§5 invariant 5). Distinct from
    /// [`Self::NoEndpoint`] because the repair is different: the model exists
    /// and is reachable, it just cannot do this job any more.
    ModalityLost {
        contract: ContractId,
        model_id: String,
        modality: &'static str,
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
            Self::CredentialRefused { .. } => KEY_REFUSED,
            Self::NoEndpoint { .. } => NO_ENDPOINT,
            Self::ModalityLost { .. } => MODALITY_LOST,
            Self::CodexNotFound(_) => CODEX_NOT_FOUND,
            Self::CodexOverrideMissing { .. } => CODEX_OVERRIDE_MISSING,
            Self::RuntimeUnavailable { .. } => RUNTIME_UNAVAILABLE,
        }
    }

    #[must_use]
    pub fn contract(&self) -> ContractId {
        match self {
            Self::Unrouted(contract)
            | Self::NoModel(contract)
            | Self::NoCredential(contract)
            | Self::CodexNotFound(contract)
            | Self::CredentialRefused { contract, .. }
            | Self::CodexOverrideMissing { contract, .. }
            | Self::RuntimeUnavailable { contract, .. }
            | Self::NoEndpoint { contract, .. }
            | Self::ModalityLost { contract, .. } => *contract,
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
            Self::CredentialRefused {
                contract,
                path,
                mode,
            } => match mode {
                Some(mode) => format!(
                    "{} ({}) is routed to OpenRouter and the key file was refused, not repaired: \
                     {path} is readable by other users (mode {mode:04o}). Treat the key as \
                     exposed, then run `chmod 600 {path}`.",
                    contract.human_label(),
                    contract.token(),
                ),
                None => format!(
                    "{} ({}) is routed to OpenRouter and the key file at {path} could not be \
                     used. Open AI settings \u{2192} OpenRouter, which names the fault and the \
                     repair; the file is never rewritten for you.",
                    contract.human_label(),
                    contract.token(),
                ),
            },
            Self::CodexNotFound(contract) => format!(
                "{} ({}) is routed to Codex and no codex executable was found on this process's \
                 PATH, in the common install locations, or on your login shell's PATH. Install \
                 it, or set local_runtimes.codex_bin in AI settings.",
                contract.human_label(),
                contract.token(),
            ),
            Self::CodexOverrideMissing {
                contract,
                path,
                reason,
            } => format!(
                "{} ({}) is routed to Codex and local_runtimes.codex_bin names {path}, which is \
                 not a runnable file ({reason}). An explicit setting is never quietly replaced \
                 by a guess: fix the path in AI settings, or clear it to search again.",
                contract.human_label(),
                contract.token(),
            ),
            Self::RuntimeUnavailable {
                contract,
                runtime_id,
                remediation,
            } => format!(
                "{} ({}) is routed to {runtime_id} and the last runtime check did not find it \
                 usable \u{2014} {remediation} \u{2014} so run the check again from AI settings \
                 \u{2192} Local models once that is done.",
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
            Self::ModalityLost {
                contract,
                model_id,
                modality,
            } => format!(
                "{} ({}) is routed to {model_id}, and the last catalog refresh no longer reports \
                 that model as accepting {modality} input. Choose a model that does in AI \
                 settings \u{2192} Routing, or route this task locally. It is never re-pointed at \
                 another model for you.",
                contract.human_label(),
                contract.token(),
            ),
        }
    }
}

/// What is known about the credential a remote route would use, as three
/// answers rather than a `bool`.
///
/// The `bool` was audit A3: `plan::openrouter_secret` collapsed every file
/// error into `None`, so a `0644` credentials file reported as "no key is
/// configured" and the user was told to add the key they already had. Refused
/// is its own fact because it has its own repair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CredentialFact {
    /// Nothing configured, and nothing imported this session.
    #[default]
    Absent,
    /// A usable key exists — from the `0600` file or from this session's
    /// environment import.
    Present,
    /// A credentials file exists and was refused rather than repaired.
    Refused {
        path: String,
        /// Present when the refusal was about the file's mode.
        mode: Option<u32>,
    },
}

impl CredentialFact {
    #[must_use]
    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present)
    }
}

/// What discovery has to say about `codex`.
///
/// `Unknown` is "nothing has looked yet", which is not grounds to refuse a job
/// — the same distinction the catalog's `None` makes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CodexFact {
    #[default]
    Unknown,
    Found,
    NotFound,
    /// `local_runtimes.codex_bin` is set and is not runnable (audit B6).
    OverrideMissing {
        path: String,
        reason: String,
    },
}

/// What a doctor report says about one local runtime.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RuntimeFact {
    /// No report, or a report that does not mention this runtime.
    #[default]
    Unmeasured,
    Available,
    Unavailable {
        remediation: String,
    },
}

/// Everything the pre-spawn check needs that the snapshot does not carry.
///
/// One struct, built once per surface, because audit A1 was two independent
/// answers to one question: the dialog's readiness badge knew about Codex
/// discovery and the doctor report and the pre-spawn preflight did not, so
/// `Blocked — codex not found` sat on the Routing tab while Start was accepted
/// and the job died inside the helper forty minutes later.
#[derive(Clone, Debug, Default)]
pub struct PreflightFacts {
    pub credential: CredentialFact,
    /// The models the last catalog fetch reported, or `None` when the catalog
    /// was never fetched. `None` is "we have not looked", which is not grounds
    /// to refuse a job.
    pub catalog_models: Option<Vec<CatalogModelFacts>>,
    pub codex: CodexFact,
    /// Doctor verdicts by [`doctor_key`], for the surfaces that have a report.
    pub local_runtimes: Vec<(String, RuntimeFact)>,
    /// How many catalog entries this contract could be pointed at under the
    /// current filters, when the caller computed it. `None` is "not computed"
    /// — the settings dialog owns the picker's filters and is the only side
    /// that can answer it.
    pub eligible_models: Option<usize>,
}

impl PreflightFacts {
    /// Facts that know only whether a credential exists. The shape the
    /// pre-`AX-1` callers had, kept for tests and for callers with nothing
    /// else to say.
    #[must_use]
    pub fn with_credential(present: bool) -> Self {
        Self {
            credential: if present {
                CredentialFact::Present
            } else {
                CredentialFact::Absent
            },
            ..Self::default()
        }
    }

    #[must_use]
    pub fn runtime(&self, key: &str) -> RuntimeFact {
        self.local_runtimes
            .iter()
            .find(|(name, _)| name == key)
            .map_or(RuntimeFact::Unmeasured, |(_, fact)| fact.clone())
    }
}

/// One route, as much of it as [`evaluate_route`] reads.
///
/// Two constructors, because the two surfaces hold the same route in two
/// shapes: the dialog has a [`ResolvedRoute`] and the pre-spawn gate has the
/// frozen [`ContractSnapshot`]. Everything after this point is one code path.
#[derive(Clone, Copy, Debug)]
pub struct RouteFacts<'a> {
    pub contract: ContractId,
    pub route_type: RouteType,
    pub runtime_id: &'a str,
    /// `None` where nothing has been chosen. Never the empty string, and never
    /// a display phrase (audit A2).
    pub model_id: Option<&'a str>,
    pub provider: Option<&'a Provider>,
}

impl<'a> RouteFacts<'a> {
    /// The row a snapshot froze. `None` for the `unrouted` placeholder, which
    /// is its own block.
    #[must_use]
    pub fn from_snapshot(entry: &'a ContractSnapshot) -> Option<Self> {
        (entry.runtime_id != "unrouted").then(|| Self {
            contract: entry.contract,
            route_type: entry.route_type,
            runtime_id: &entry.runtime_id,
            model_id: (!entry.model_id.is_empty()).then_some(entry.model_id.as_str()),
            provider: entry.provider_constraints.as_ref(),
        })
    }

    /// What the dialog is showing. `None` for a contract with no route.
    #[must_use]
    pub fn from_resolved(resolved: &'a ResolvedRoute) -> Option<Self> {
        let route = resolved.route.as_ref()?;
        Some(Self {
            contract: resolved.contract,
            route_type: route.route_type,
            runtime_id: &route.runtime_id,
            model_id: route.model_id.as_deref().filter(|id| !id.is_empty()).or(
                match route.route_type {
                    // The two route types that resolve their own model, and
                    // record what they resolved. `model_id_recorded` says the
                    // same thing about the snapshot side.
                    RouteType::Codex => Some(CODEX_DEFAULT_LABEL),
                    RouteType::Builtin | RouteType::LocalProc => Some(&route.runtime_id),
                    RouteType::OpenRouter => None,
                },
            ),
            provider: route.provider.as_ref(),
        })
    }
}

/// One route's verdict: the third state is the point.
///
/// `Unknown` is "nothing has measured this yet", and it is neither a block nor
/// a promise. Collapsing it into either is how a readiness badge lies — a
/// confident "not found" for a probe still running, or a green Ready for a
/// runtime nobody looked at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteVerdict {
    Ready,
    Unknown(String),
    Blocked(ExecutionBlock),
}

impl RouteVerdict {
    #[must_use]
    pub fn block(self) -> Option<ExecutionBlock> {
        match self {
            Self::Blocked(block) => Some(block),
            _ => None,
        }
    }
}

/// **The** readiness answer. One function, two call sites: the settings
/// dialog's badge and the pre-spawn gate (audit A1).
///
/// The order is the order the pieces are needed in — an executor, then a
/// credential where one is required, then something to send, then somewhere
/// that would take it — and the badge names the first missing one, because
/// four blocked contracts is still one thing to go and fix first.
#[must_use]
pub fn evaluate_route(route: &RouteFacts<'_>, facts: &PreflightFacts) -> RouteVerdict {
    if !route
        .contract
        .route_is_implemented(route.route_type, route.runtime_id, route.model_id)
    {
        return RouteVerdict::Blocked(ExecutionBlock::UnsupportedRoute {
            contract: route.contract,
            route_type: route.route_type,
            runtime_id: route.runtime_id.to_string(),
        });
    }
    // 1. The credential, where the route type needs one. Codex authenticates
    //    itself and the local lanes open no socket, so only OpenRouter does.
    if route.route_type == RouteType::OpenRouter {
        match &facts.credential {
            CredentialFact::Present => {}
            CredentialFact::Absent => {
                return RouteVerdict::Blocked(ExecutionBlock::NoCredential(route.contract));
            }
            CredentialFact::Refused { path, mode } => {
                return RouteVerdict::Blocked(ExecutionBlock::CredentialRefused {
                    contract: route.contract,
                    path: path.clone(),
                    mode: *mode,
                });
            }
        }
    }
    // 2. Something to send. `RouteFacts` has already applied the rule that a
    //    Codex or local route resolves its own model.
    if route.model_id.is_none() {
        return RouteVerdict::Blocked(ExecutionBlock::NoModel(route.contract));
    }
    // 3. Whatever else that route type needs to exist.
    match route.route_type {
        RouteType::Builtin => RouteVerdict::Ready,
        RouteType::Codex => match &facts.codex {
            CodexFact::Found => RouteVerdict::Ready,
            CodexFact::Unknown => RouteVerdict::Unknown("Looking for codex".to_string()),
            CodexFact::NotFound => {
                RouteVerdict::Blocked(ExecutionBlock::CodexNotFound(route.contract))
            }
            CodexFact::OverrideMissing { path, reason } => {
                RouteVerdict::Blocked(ExecutionBlock::CodexOverrideMissing {
                    contract: route.contract,
                    path: path.clone(),
                    reason: reason.clone(),
                })
            }
        },
        RouteType::LocalProc => {
            let Some(key) = doctor_key(route.runtime_id) else {
                return RouteVerdict::Unknown("Not probed".to_string());
            };
            match facts.runtime(key) {
                RuntimeFact::Available => RouteVerdict::Ready,
                RuntimeFact::Unmeasured => RouteVerdict::Unknown("Run doctor".to_string()),
                RuntimeFact::Unavailable { remediation } => {
                    RouteVerdict::Blocked(ExecutionBlock::RuntimeUnavailable {
                        contract: route.contract,
                        runtime_id: route.runtime_id.to_string(),
                        remediation,
                    })
                }
            }
        }
        RouteType::OpenRouter => openrouter_verdict(route, facts),
    }
}

/// The remote half of [`evaluate_route`]: constraints, then the catalog.
fn openrouter_verdict(route: &RouteFacts<'_>, facts: &PreflightFacts) -> RouteVerdict {
    if let Some(provider) = route.provider {
        if let Some(constraint) = unsatisfiable_constraint(provider) {
            return RouteVerdict::Blocked(ExecutionBlock::NoEndpoint {
                contract: route.contract,
                constraint,
            });
        }
    }
    if facts.eligible_models == Some(0) {
        return RouteVerdict::Blocked(ExecutionBlock::NoEndpoint {
            contract: route.contract,
            constraint: "the cached catalog offers no model this contract can use under the \
                         current filters"
                .to_string(),
        });
    }
    let model_id = route.model_id.unwrap_or_default();
    // An absent catalog is "we have not looked", not "there is nothing" — the
    // same distinction the dialog's never-fetched badge makes. Two facts can
    // say a fetch happened and they are held by different callers: the gate
    // parses the cache into rows, the dialog counts what its own filters leave.
    // Either is evidence that somebody looked; neither being present is the
    // only state that means nobody has.
    let Some(catalog) = &facts.catalog_models else {
        return if facts.eligible_models.is_some() {
            RouteVerdict::Ready
        } else {
            RouteVerdict::Unknown("Catalog never fetched".to_string())
        };
    };
    let Some(model) = catalog.iter().find(|model| model.id == model_id) else {
        return RouteVerdict::Blocked(ExecutionBlock::NoEndpoint {
            contract: route.contract,
            constraint: format!("the last catalog fetch does not list {model_id}"),
        });
    };
    // §5 invariant 5: membership is not capability. A model that is still
    // listed but no longer reports the modality this contract sends has
    // invalidated the route, and saying so here is the whole point of a
    // preflight — the alternative is a job that spawns, uploads and fails at
    // submit time.
    if let Some(modality) = route.contract.required_input_modality() {
        if model.refuses_input(modality) {
            return RouteVerdict::Blocked(ExecutionBlock::ModalityLost {
                contract: route.contract,
                model_id: model_id.to_string(),
                modality,
            });
        }
    }
    RouteVerdict::Ready
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
/// What *is* decidable from the catalog is both halves of the model's identity,
/// and the second half used to be missing: §5 invariant 5 says "a route that
/// loses its required modality is invalid", and checking membership by id alone
/// reads Ready for a model that a refreshed catalog still lists but no longer
/// says takes audio. That job then failed at submit time, which is exactly the
/// late failure this function exists to prevent. Membership and
/// [`ContractId::required_input_modality`] are now both checked, and they are
/// separate blocks because they are separate repairs.
///
/// So the ZDR rule is enforced in two places rather than pretended in one.
/// Here, a `zdr_required` route whose provider allow-list is emptied by
/// `ignore`, by a disjoint `order` with fallbacks off, or by a zero price bound
/// is refused **before** anything spawns and the message names ZDR alongside
/// the constraint that emptied it. Beyond that, `provider.zdr` is sent on the
/// request and OpenRouter refuses rather than substituting, and the helper
/// surfaces that refusal verbatim. Neither path ever weakens the constraint.
///
/// ## One evaluator
///
/// The decision itself is [`evaluate_route`], and this function is the fold of
/// it over a frozen graph. The settings dialog's readiness badge calls the same
/// evaluator on its own resolved routes, which is audit A1's repair: before it,
/// the badge could say `Blocked — codex not found` while this function had no
/// Codex arm at all and accepted the Start it was standing next to.
#[must_use]
pub fn preflight(snapshot: &ExecutionSnapshot, facts: &PreflightFacts) -> Vec<ExecutionBlock> {
    snapshot
        .contracts
        .iter()
        .filter_map(|entry| match RouteFacts::from_snapshot(entry) {
            None => Some(ExecutionBlock::Unrouted(entry.contract)),
            Some(route) => evaluate_route(&route, facts).block(),
        })
        .collect()
}

/// The consent sentence for one resolved graph: exactly what leaves this
/// computer, read off the snapshot rather than off the workflow's name.
///
/// Audit A10. `AssistMode::data_boundary()` is a static per-mode string drawn
/// directly above the resolved route list, so a user who re-pointed a contract
/// read a sentence about the routes the built-in profile would have used. It
/// was latent only because `implemented_route_types` happened to block the
/// contradicting route — and audit A4 made it false outright, because "Zero
/// Data Retention is requested" is now a property of the snapshot rather than
/// a constant.
#[must_use]
pub fn consent_sentence(snapshot: &ExecutionSnapshot) -> String {
    let names = |rank: u8| -> Vec<&'static str> {
        snapshot
            .contracts
            .iter()
            .filter(|entry| entry.boundary_applied.rank() == rank)
            .map(|entry| entry.contract.human_label())
            .collect()
    };
    let destinations = |rank: u8| -> Vec<String> {
        let mut ids: Vec<String> = snapshot
            .contracts
            .iter()
            .filter(|entry| entry.boundary_applied.rank() == rank)
            .map(|entry| entry.runtime_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let audio = names(2);
    let text = names(1);
    if audio.is_empty() && text.is_empty() {
        return "Nothing leaves this computer: every task in this job runs locally.".to_string();
    }
    let mut parts = Vec::new();
    if !audio.is_empty() {
        parts.push(format!(
            "Track audio is sent to {} for {}{}.",
            destinations(2).join(", "),
            audio.join(", "),
            if snapshot.requires_zdr() {
                ", with zero data retention requested"
            } else {
                ", and zero data retention is not requested"
            },
        ));
    }
    if !text.is_empty() {
        parts.push(format!(
            "Derived text is sent to {} for {}; no audio.",
            destinations(1).join(", "),
            text.join(", "),
        ));
    }
    parts.push("Nothing else leaves this computer.".to_string());
    parts.join(" ")
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
            // Audit A5. This assertion did not exist, and every row of this
            // graph carried `audio_scope: whole-track` beside
            // `boundary_applied: local-only` — because mms-ctc and
            // whisper.cpp's overlay rows say `whole-track` about the model in
            // the abstract, and a lane that opens no socket sends nothing
            // anywhere. A snapshot is read to answer "what left this machine".
            assert_eq!(
                entry.audio_scope,
                Some(AudioScope::None),
                "{} records audio leaving a local-only lane",
                entry.contract.token(),
            );
        }
        assert!(preflight(&snapshot, &PreflightFacts::with_credential(false)).is_empty());
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
        let blocks = preflight(&snapshot, &PreflightFacts::with_credential(false));
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

    /// Audit A10 + A4. The consent sentence is a function of the resolved
    /// graph, so re-pointing a contract changes it — and the ZDR clause is now
    /// a claim about what will be sent rather than a constant.
    #[test]
    fn the_consent_sentence_is_read_off_the_graph_and_moves_with_it() {
        let local = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            true,
            &facts(),
        );
        assert_eq!(
            consent_sentence(&local),
            "Nothing leaves this computer: every task in this job runs locally."
        );

        // The same workflow with no authored sheet composes the Codex wording
        // review, which sends derived text and no audio.
        let wording = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            false,
            &facts(),
        );
        let sentence = consent_sentence(&wording);
        assert!(
            sentence.contains("Derived text is sent to codex"),
            "{sentence}"
        );
        assert!(sentence.contains("no audio"), "{sentence}");
        assert!(!sentence.contains("Track audio"), "{sentence}");

        let mimo = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let sentence = consent_sentence(&mimo);
        assert!(
            sentence.contains("Track audio is sent to openrouter"),
            "{sentence}"
        );
        assert!(
            sentence.contains("with zero data retention requested"),
            "{sentence}"
        );

        // And with ZDR turned off on the route, the sentence says so rather
        // than repeating the promise the mode string used to make.
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        let mut provider = Provider::defaults_for(ContractId::Semantic);
        provider.zdr_required = false;
        route.provider = Some(provider);
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Semantic, route)]),
        });
        settings.active_profile = "studio".to_string();
        let relaxed = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        assert!(!relaxed.requires_zdr());
        let sentence = consent_sentence(&relaxed);
        assert!(
            sentence.contains("zero data retention is not requested"),
            "{sentence}"
        );
    }

    /// Audit A5, the general statement: **no** row of **any** workflow may
    /// record a scope wider than its own boundary admits. The boundary is the
    /// ceiling and the overlay may only narrow it.
    #[test]
    fn no_snapshot_row_records_audio_leaving_a_lane_that_does_not_send_it() {
        for kind in [
            WorkflowKind::Lyrics,
            WorkflowKind::Sections,
            WorkflowKind::Mimo,
            WorkflowKind::All,
        ] {
            for reference in [true, false] {
                let snapshot = resolve(&AssistSettings::default(), kind, reference, &facts());
                let mut audio_rows = 0;
                for entry in &snapshot.contracts {
                    let scope = entry.audio_scope.expect("every row states a scope");
                    let ceiling = if entry.boundary_applied.rank() >= 2 {
                        AudioScope::WholeTrack
                    } else {
                        AudioScope::None
                    };
                    assert!(
                        scope.rank() <= ceiling.rank(),
                        "{kind:?}/{reference} {}: {} under {}",
                        entry.contract.token(),
                        scope.token(),
                        entry.boundary_applied.token(),
                    );
                    audio_rows += usize::from(scope.rank() > 0);
                }
                // And the check has teeth: the audio-leaving workflows really
                // do still record whole-track, so this is not passing because
                // everything was flattened to `none`.
                assert_eq!(
                    audio_rows > 0,
                    snapshot.sends_audio_off_machine(),
                    "{kind:?}/{reference}",
                );
            }
        }
        let mimo = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        assert_eq!(
            mimo.contract(ContractId::Semantic).unwrap().audio_scope,
            Some(AudioScope::WholeTrack),
        );
    }

    /// Audit A2. `ExecutionBlock::NoModel` was unreachable by construction:
    /// the snapshot recorded `model_label()`, which for a remote route with no
    /// model chosen is the phrase `not chosen`, so `model_id.is_empty()` was
    /// never true. The job then either asked OpenRouter for a model called
    /// "not chosen" or blocked with a misleading `NoEndpoint`.
    #[test]
    fn a_remote_route_with_no_model_blocks_as_no_model_and_records_nothing() {
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        route.model_id = None;
        settings.profiles.push(Profile {
            id: "studio".to_string(),
            label: "Studio".to_string(),
            routes: BTreeMap::from([(ContractId::Semantic, route)]),
        });
        settings.active_profile = "studio".to_string();
        let snapshot = resolve(&settings, WorkflowKind::Mimo, true, &facts());
        let entry = snapshot.contract(ContractId::Semantic).unwrap();
        assert_eq!(
            entry.model_id, "",
            "a `model_id` field holding a sentence is not an identity"
        );
        let blocks = preflight(&snapshot, &PreflightFacts::with_credential(true));
        assert_eq!(blocks, vec![ExecutionBlock::NoModel(ContractId::Semantic)]);
        assert_eq!(blocks[0].label(), NO_MODEL);

        // And the picker keeps its own word for the same state, because a
        // column has to say something.
        let resolved = resolve_route(&settings, ContractId::Semantic);
        assert_eq!(resolved.model_label(), "not chosen");
        assert_eq!(resolved.model_id_recorded(), "");
        // The two labels that are *not* placeholders still round-trip.
        let codex = resolve_route(&AssistSettings::default(), ContractId::Wording);
        assert_eq!(codex.model_id_recorded(), CODEX_DEFAULT_LABEL);
        let local = resolve_route(&AssistSettings::default(), ContractId::Coarse);
        assert_eq!(local.model_id_recorded(), "whisper.cpp");
    }

    /// Audit A3. A `0644` credentials file is not the same fact as no key, and
    /// the sentence must not tell a user to add the key in front of them.
    #[test]
    fn a_refused_credentials_file_blocks_with_the_chmod_rather_than_add_a_key() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                credential: CredentialFact::Refused {
                    path: "/home/x/.config/musializer/credentials.json".to_string(),
                    mode: Some(0o644),
                },
                ..PreflightFacts::default()
            },
        );
        assert_eq!(
            blocks,
            vec![ExecutionBlock::CredentialRefused {
                contract: ContractId::Semantic,
                path: "/home/x/.config/musializer/credentials.json".to_string(),
                mode: Some(0o644),
            }]
        );
        assert_eq!(blocks[0].label(), KEY_REFUSED);
        let sentence = blocks[0].sentence();
        assert!(sentence.contains("chmod 600"), "{sentence}");
        assert!(sentence.contains("0644"), "{sentence}");
        assert!(sentence.contains("credentials.json"), "{sentence}");
        assert!(
            !sentence.contains("no key is configured"),
            "the NoCredential sentence is the wrong repair here: {sentence}"
        );
        // A file refused for a reason other than its mode still names itself.
        let other = ExecutionBlock::CredentialRefused {
            contract: ContractId::Semantic,
            path: "/tmp/creds.json".to_string(),
            mode: None,
        };
        assert!(other.sentence().contains("/tmp/creds.json"));
    }

    /// Audit A1 + B6. Preflight had no Codex arm at all, so the Routing tab's
    /// `Blocked — codex not found` stood beside a Start that was accepted; and
    /// a set-but-missing `codex_bin` was omitted from the argv, which sent the
    /// helper hunting for a different `codex` on `PATH`.
    #[test]
    fn the_codex_lane_blocks_for_both_discovery_failures_and_waits_for_neither() {
        // TC-WORDING is the Codex route, and it is composed when the track has
        // no authored sheet.
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            false,
            &facts(),
        );
        assert!(snapshot.contract(ContractId::Wording).is_some());

        let blocks = |codex: CodexFact| {
            preflight(
                &snapshot,
                &PreflightFacts {
                    codex,
                    ..PreflightFacts::default()
                },
            )
        };
        assert!(
            blocks(CodexFact::Unknown).is_empty(),
            "nothing has looked yet, which is not grounds to refuse"
        );
        assert!(blocks(CodexFact::Found).is_empty());
        assert_eq!(
            blocks(CodexFact::NotFound),
            vec![ExecutionBlock::CodexNotFound(ContractId::Wording)]
        );
        assert_eq!(blocks(CodexFact::NotFound)[0].label(), CODEX_NOT_FOUND);

        let missing = blocks(CodexFact::OverrideMissing {
            path: "/opt/nowhere/codex".to_string(),
            reason: "not a runnable file".to_string(),
        });
        assert_eq!(missing[0].label(), CODEX_OVERRIDE_MISSING);
        let sentence = missing[0].sentence();
        assert!(
            sentence.contains("/opt/nowhere/codex"),
            "a wrong path with the path left out is not a repair: {sentence}"
        );
        assert!(sentence.contains("codex_bin"), "{sentence}");
    }

    /// Audit A1. The doctor arm, in both directions: an unmeasured runtime is
    /// not a refusal, and a measured-and-broken one is.
    #[test]
    fn a_doctor_reported_runtime_failure_blocks_the_local_lane() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            true,
            &facts(),
        );
        assert!(preflight(&snapshot, &PreflightFacts::default()).is_empty());
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                local_runtimes: vec![(
                    "whisper".to_string(),
                    RuntimeFact::Unavailable {
                        remediation: "download the model into models/whisper".to_string(),
                    },
                )],
                ..PreflightFacts::default()
            },
        );
        assert_eq!(
            blocks,
            vec![ExecutionBlock::RuntimeUnavailable {
                contract: ContractId::Coarse,
                runtime_id: "whisper.cpp".to_string(),
                remediation: "download the model into models/whisper".to_string(),
            }]
        );
        assert_eq!(blocks[0].label(), RUNTIME_UNAVAILABLE);
        assert!(blocks[0].sentence().contains("download the model"));
        // And the report is read through the same key mapping the snapshot's
        // `runtime_version` uses, or a lane would report Ready beside a blank
        // version.
        assert_eq!(doctor_key("whisper.cpp"), Some("whisper"));
        assert_eq!(
            parse_doctor_reading(
                br#"{"schema_version":"musializer.doctor/v1",
                     "runtimes":{"whisper":{"state":"missing","remediation":"install it"},
                     "mms_ctc_aligner":{"state":"ok"}}}"#
            )
            .facts()
            .runtimes,
            vec![
                ("mms_ctc_aligner".to_string(), RuntimeFact::Available),
                (
                    "whisper".to_string(),
                    RuntimeFact::Unavailable {
                        remediation: "install it".to_string()
                    }
                ),
            ]
        );
        // A report that mentions no state at all contributes nothing rather
        // than an invented refusal.
        assert!(parse_doctor_reading(
            br#"{"schema_version":"musializer.doctor/v1","runtimes":{"whisper":{}}}"#
        )
        .facts()
        .runtimes
        .is_empty());
    }

    /// Every block spells out a repair. A block whose sentence does not name
    /// its own contract, or is short enough to be a label, is a toast nobody
    /// can act on.
    #[test]
    fn every_block_variant_states_a_repair_and_names_its_contract() {
        let blocks = [
            ExecutionBlock::Unrouted(ContractId::Align),
            ExecutionBlock::UnsupportedRoute {
                contract: ContractId::Coarse,
                route_type: RouteType::OpenRouter,
                runtime_id: "openrouter".to_string(),
            },
            ExecutionBlock::NoModel(ContractId::Semantic),
            ExecutionBlock::NoCredential(ContractId::Semantic),
            ExecutionBlock::CredentialRefused {
                contract: ContractId::Semantic,
                path: "/tmp/creds.json".to_string(),
                mode: Some(0o600),
            },
            ExecutionBlock::CredentialRefused {
                contract: ContractId::Semantic,
                path: "/tmp/creds.json".to_string(),
                mode: None,
            },
            ExecutionBlock::NoEndpoint {
                contract: ContractId::Semantic,
                constraint: "provider.only is empty".to_string(),
            },
            ExecutionBlock::ModalityLost {
                contract: ContractId::Semantic,
                model_id: "xiaomi/mimo-v2.5".to_string(),
                modality: "audio",
            },
            ExecutionBlock::CodexNotFound(ContractId::Wording),
            ExecutionBlock::CodexOverrideMissing {
                contract: ContractId::Wording,
                path: "/opt/nowhere/codex".to_string(),
                reason: "not a runnable file".to_string(),
            },
            ExecutionBlock::RuntimeUnavailable {
                contract: ContractId::Coarse,
                runtime_id: "whisper.cpp".to_string(),
                remediation: "install whisper.cpp".to_string(),
            },
        ];
        let mut labels = Vec::new();
        for block in &blocks {
            let sentence = block.sentence();
            assert!(
                sentence.contains(block.contract().token()),
                "{sentence} does not name its contract"
            );
            assert!(sentence.len() > 60, "too short to be a repair: {sentence}");
            assert!(sentence.ends_with('.'), "{sentence}");
            assert!(!block.label().is_empty());
            labels.push(block.label());
        }
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(
            labels.len(),
            10,
            "two variants sharing one badge is one word for two repairs"
        );
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
        let blocks = preflight(&snapshot, &PreflightFacts::with_credential(true));
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
        let blocks = preflight(&snapshot, &PreflightFacts::with_credential(true));
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
        let (revision, models) = parse_catalog_facts(
            br#"{"schema_version":"musializer.openrouter-catalog/v1",
                 "fetched_at_utc":"2026-08-05T10:00:00Z",
                 "models":[{"id":"xiaomi/mimo-v2.5","input_modalities":["audio","text"]},
                           {"id":"openai/gpt-4o"},{"name":"no id"}]}"#,
        )
        .expect("a well-formed catalog");
        assert_eq!(
            revision,
            "musializer.openrouter-catalog/v1@2026-08-05T10:00:00Z"
        );
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, vec!["xiaomi/mimo-v2.5", "openai/gpt-4o"]);
        assert_eq!(models[0].input_modalities, vec!["audio", "text"]);
        // A row that reports no modalities at all is "not reported", and must
        // never be read as a refusal: `tools/provider_catalog.py` normalizes a
        // missing `architecture` block to an empty list.
        assert!(models[1].input_modalities.is_empty());
        assert!(!models[1].refuses_input("audio"));
        assert!(!models[0].refuses_input("audio"));
        assert!(models[0].refuses_input("image"));
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
        let facts = parse_doctor_reading(
            br#"{"schema_version":"musializer.doctor/v1","runtimes":{
                "whisper":{"state":"available","version":"whisper.cpp 1.8.6","model_sha256":"beef"},
                "mms_ctc_aligner":{"state":"missing"}}}"#,
        )
        .facts();
        assert_eq!(
            facts.runtime_versions,
            vec![("whisper".to_string(), "whisper.cpp 1.8.6".to_string())]
        );
        assert_eq!(
            facts.model_digests,
            vec![("whisper".to_string(), "beef".to_string())]
        );
        // A v1 report with no runtimes block is still a v1 report.
        assert_eq!(
            parse_doctor_reading(br#"{"schema_version":"musializer.doctor/v1"}"#),
            DoctorReading::Report(DoctorFacts::default())
        );
    }

    /// Audit B5, and the reason it is a whole enum rather than an empty list:
    /// a report from another version, a report whose `runtimes` key was
    /// renamed, and a report nobody took are three different facts that used to
    /// produce one picture — an empty runtime list under "Doctor finished;
    /// runtime identities updated."
    #[test]
    fn a_foreign_doctor_schema_is_a_named_state_and_not_a_silent_parse() {
        assert_eq!(
            parse_doctor_reading(
                br#"{"schema_version":"musializer.doctor/v2","runtimes":{
                     "whisper":{"state":"available","version":"1.8.6"}}}"#
            ),
            DoctorReading::ForeignSchema("musializer.doctor/v2".to_string())
        );
        // Nothing from a foreign document reaches the snapshot's provenance.
        assert_eq!(
            parse_doctor_reading(
                br#"{"schema_version":"musializer.doctor/v2","runtimes":{
                     "whisper":{"state":"available","version":"1.8.6"}}}"#
            )
            .facts(),
            DoctorFacts::default()
        );
        // A JSON object with no schema at all is the renamed-key case, and it
        // is the one that used to read as a clean empty report.
        assert_eq!(
            parse_doctor_reading(br#"{"runtimes":{"whisper":{"state":"available"}}}"#),
            DoctorReading::ForeignSchema("none declared".to_string())
        );
        assert!(matches!(
            parse_doctor_reading(b"{ broken"),
            DoctorReading::Unreadable(_)
        ));
        assert!(matches!(
            parse_doctor_reading(b"[]"),
            DoctorReading::Unreadable(_)
        ));
        // Every state says which it is, in one greppable token.
        let tokens: Vec<String> = [
            DoctorReading::NotTaken,
            DoctorReading::Unreadable("not JSON".to_string()),
            DoctorReading::ForeignSchema("musializer.doctor/v2".to_string()),
            DoctorReading::Report(DoctorFacts::default()),
        ]
        .iter()
        .map(DoctorReading::token)
        .collect();
        assert_eq!(
            tokens
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        assert!(tokens[2].contains("musializer.doctor/v2"));
    }

    #[test]
    fn a_never_fetched_catalog_does_not_refuse_a_job() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        assert!(preflight(&snapshot, &PreflightFacts::with_credential(true)).is_empty());
        // But a catalog that *was* fetched and does not list the model does.
        // This case used to be the *only* catalog check — membership by id —
        // which is what let a model that lost its modality read Ready; the two
        // tests below are the other half (§5 invariant 5).
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                catalog_models: Some(vec![catalog_row(
                    "openai/gpt-4o-audio-preview",
                    &["audio", "text"],
                )]),
                ..PreflightFacts::with_credential(true)
            },
        );
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], ExecutionBlock::NoEndpoint { .. }));
        assert!(blocks[0].sentence().contains("xiaomi/mimo-v2.5"));
    }

    fn catalog_row(id: &str, inputs: &[&str]) -> CatalogModelFacts {
        CatalogModelFacts {
            id: id.to_string(),
            input_modalities: inputs.iter().map(|entry| (*entry).to_string()).collect(),
        }
    }

    #[test]
    fn a_listed_model_that_still_takes_audio_is_ready() {
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        assert!(preflight(
            &snapshot,
            &PreflightFacts {
                catalog_models: Some(vec![catalog_row("xiaomi/mimo-v2.5", &["audio", "text"])]),
                ..PreflightFacts::with_credential(true)
            },
        )
        .is_empty());
        // And a row whose modalities were never reported is not evidence of
        // loss, so it does not refuse either.
        assert!(preflight(
            &snapshot,
            &PreflightFacts {
                catalog_models: Some(vec![catalog_row("xiaomi/mimo-v2.5", &[])]),
                ..PreflightFacts::with_credential(true)
            },
        )
        .is_empty());
    }

    #[test]
    fn a_model_that_lost_its_required_modality_refuses_by_name() {
        // The defect this test exists for: the model is still in the catalog,
        // so an id-only membership check reads Ready and the job fails at
        // submit time instead of at preflight (§5 invariant 5).
        let snapshot = resolve(
            &AssistSettings::default(),
            WorkflowKind::Mimo,
            true,
            &facts(),
        );
        let blocks = preflight(
            &snapshot,
            &PreflightFacts {
                catalog_models: Some(vec![catalog_row("xiaomi/mimo-v2.5", &["text"])]),
                ..PreflightFacts::with_credential(true)
            },
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            ExecutionBlock::ModalityLost {
                contract: ContractId::Semantic,
                model_id: "xiaomi/mimo-v2.5".to_string(),
                modality: "audio",
            }
        );
        // A distinct, actionable sentence: not "not found", and it names both
        // the model and what it stopped being able to do.
        let sentence = blocks[0].sentence();
        assert!(sentence.contains("xiaomi/mimo-v2.5"), "{sentence}");
        assert!(sentence.contains("accepting audio input"), "{sentence}");
        assert!(!sentence.contains("does not list"), "{sentence}");
        assert_eq!(blocks[0].label(), MODALITY_LOST);
        assert_eq!(blocks[0].contract(), ContractId::Semantic);
    }

    #[test]
    fn a_text_only_contract_does_not_require_audio() {
        // TC-WORDING and TC-PLAN send derived JSON, so a text-only model is
        // correct for them and must not be refused by the audio rule.
        assert_eq!(ContractId::Wording.required_input_modality(), None);
        assert_eq!(ContractId::Plan.required_input_modality(), None);
        assert_eq!(
            ContractId::Semantic.required_input_modality(),
            Some("audio")
        );
        assert_eq!(ContractId::Coarse.required_input_modality(), Some("audio"));
        assert_eq!(ContractId::Verify.required_input_modality(), Some("audio"));
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
