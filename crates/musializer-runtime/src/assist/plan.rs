//! Gathering the impure facts an execution snapshot needs, and freezing it.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §5 and §6. The decision half is
//! [`musializer_core::assist::execution`], which opens no files; this is the
//! part that reads `assist.json`, the `0600` credentials store, the discovery
//! caches and the wall clock, and then writes the snapshot into the job folder.
//!
//! ## Why the snapshot is a file rather than an argument
//!
//! It is provenance, and provenance that only exists as an argv is provenance a
//! crash loses. Writing it into the job's own output directory before the child
//! starts also makes §5 invariant 3 checkable from the outside: the bytes are on
//! disk, and a settings edit mid-job cannot change them because nothing rewrites
//! the file. The helper is handed the *path*, not the content, for the same
//! reason no flag ever takes a key (E2) — an argv is the wrong place for a
//! record this size.
//!
//! ## Credentials
//!
//! [`openrouter_secret`] reads the `0600` file. The session credential — the one
//! imported from the environment at startup — stays with its owner in
//! `musializer-app`, because a second copy of a key is a second lifetime to
//! reason about (§3, "one owner"). The caller picks between them and hands the
//! chosen one to `AssistSpec`, which puts it in **one** child's environment and
//! nowhere else.

use std::path::{Path, PathBuf};

use musializer_core::assist::contracts::RouteType;
use musializer_core::assist::credentials::CredentialStore;
use musializer_core::assist::execution::{
    self, ExecutionBlock, ExecutionFacts, ExecutionSnapshot, PreflightFacts, WorkflowKind,
};
use musializer_core::assist::secret::Secret;
use musializer_core::assist::settings::{AssistSettings, LocalRuntimes};

use super::discover;
use super::files::{self, AssistFileError};

/// The file a job's snapshot is written to, inside its own output directory.
pub const SNAPSHOT_FILE_NAME: &str = "assist-execution.json";

/// The size cap on a discovery cache, matching `ui/assist_settings.rs`.
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

/// Where the credential that would authorize a remote route comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    /// Nothing configured. A remote route blocks.
    None,
    /// The `0600` credentials file.
    File,
    /// Imported from the environment at startup, held for this run only.
    Session,
    /// A credentials file is there and was refused rather than repaired (§3).
    ///
    /// This variant is audit A3. [`openrouter_secret`] collapsed every
    /// `AssistFileError` into `None` with `.ok()??` — the `BeatUpdate` shape —
    /// so a `0644` file and no file at all were the same fact, and the block
    /// the user was shown told them to add a key they already had. The mode is
    /// carried because the repair is `chmod 600` and they need to know what it
    /// is now.
    Refused { path: String, mode: Option<u32> },
}

impl CredentialSource {
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            Self::None => "none".to_string(),
            Self::File => "file".to_string(),
            Self::Session => "session".to_string(),
            Self::Refused {
                mode: Some(mode), ..
            } => format!("refused({mode:04o})"),
            Self::Refused { mode: None, .. } => "refused".to_string(),
        }
    }

    /// Whether a job could actually authorize a remote route with this.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::File | Self::Session)
    }

    /// The same fact in the vocabulary the one evaluator reads.
    #[must_use]
    pub fn fact(&self) -> execution::CredentialFact {
        match self {
            Self::File | Self::Session => execution::CredentialFact::Present,
            Self::None => execution::CredentialFact::Absent,
            Self::Refused { path, mode } => execution::CredentialFact::Refused {
                path: path.clone(),
                mode: *mode,
            },
        }
    }
}

/// What the `0600` store had to say, with the refusals kept apart from the
/// absence (audit A3).
#[derive(Debug)]
pub enum CredentialLookup {
    Found(Secret),
    Absent,
    Refused { path: String, mode: Option<u32> },
}

/// One job's frozen route graph, plus everything the confirmation has to say
/// about it.
#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub snapshot: ExecutionSnapshot,
    /// Empty means the job may start. Every entry names one repair.
    pub blocks: Vec<ExecutionBlock>,
    /// The codex discovery this resolution ran, when the graph has a Codex
    /// route. Carried so the spawn uses the **same** answer the preflight
    /// judged — resolving it a second time at the spawn is how a job ends up
    /// running a different binary from the one that was cleared (audit B6).
    pub codex: Option<discover::Discovery>,
    pub credential_source: CredentialSource,
    /// A settings file that failed to load. The plan still resolves — from the
    /// built-in `recommended` profile — but the user is told, because a job
    /// running on defaults while a file sits there unread is exactly the silent
    /// substitution §2 refuses.
    pub settings_error: Option<String>,
    pub settings_path: Option<PathBuf>,
    /// The account label this profile's credential is stored under. Never the
    /// secret, and never displayed (§4 E4).
    pub credential_lookup: String,
    /// Contracts whose stored fallback policy is `ask` and which this build
    /// applied as `none`. The confirmation names them, because a policy the user
    /// chose and the application silently declined to honour is the one thing a
    /// pre-Start summary must not leave out.
    pub ask_resolved_to_none: Vec<musializer_core::assist::contracts::ContractId>,
    /// `local_runtimes` from `assist.json` (§2), which the helper takes as
    /// flags. Carried on the plan so the panel does not read the settings file
    /// a second time to build the spec.
    pub local_runtimes: LocalRuntimes,
}

impl ExecutionPlan {
    #[must_use]
    pub fn can_start(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The first refusal, which is what the panel shows: four blocked contracts
    /// is still one thing to go and fix first.
    #[must_use]
    pub fn first_block(&self) -> Option<&ExecutionBlock> {
        self.blocks.first()
    }

    /// One greppable line per job, for the report a capture carries.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "profile={} contracts={} remote={} audio-leaves={} credential={} ask-as-none={} \
             blocks={} snapshot={}",
            self.snapshot.profile_id,
            self.snapshot
                .contracts
                .iter()
                .map(execution::ContractSnapshot::compact)
                .collect::<Vec<_>>()
                .join(","),
            self.snapshot.has_remote_route(),
            self.snapshot.sends_audio_off_machine(),
            self.credential_source.token(),
            if self.ask_resolved_to_none.is_empty() {
                "none".to_string()
            } else {
                self.ask_resolved_to_none
                    .iter()
                    .map(|contract| contract.token())
                    .collect::<Vec<_>>()
                    .join(",")
            },
            if self.blocks.is_empty() {
                "none".to_string()
            } else {
                self.blocks
                    .iter()
                    .map(|block| format!("{}:{}", block.contract().token(), block.label()))
                    .collect::<Vec<_>>()
                    .join(",")
            },
            self.snapshot.snapshot_schema,
        )
    }
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

fn cache_dir() -> Option<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_CACHE_HOME") {
        if !base.is_empty() {
            return Some(PathBuf::from(base).join("musializer"));
        }
    }
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".cache/musializer"))
}

/// `(revision, models)` from the OpenRouter catalog cache, or `None` when it
/// was never fetched.
///
/// **Absent is "we have not looked", not "there are no models".** That is the
/// same distinction the dialog's `never fetched` badge makes and it is why
/// [`execution::preflight`] refuses a job only when a catalog *was* fetched and
/// either does not list the model or no longer credits it with the modality the
/// contract needs (§5 invariant 5).
fn catalog_facts() -> Option<(String, Vec<execution::CatalogModelFacts>)> {
    let path = cache_dir()?.join("openrouter-models-v1.json");
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > MAX_CACHE_BYTES {
        return None;
    }
    execution::parse_catalog_facts(&std::fs::read(&path).ok()?)
}

/// `(runtime versions, model digests)` from a doctor report, when one has been
/// taken. Nothing runs the doctor here: an unmeasured runtime records `null`
/// rather than a guessed version.
#[allow(
    clippy::type_complexity,
    reason = "two sorted association lists; the core parser names them the same way"
)]
fn doctor_facts(report_path: Option<&Path>) -> (Vec<(String, String)>, Vec<(String, String)>) {
    match doctor_bytes(report_path) {
        Some(bytes) => execution::parse_doctor_facts(&bytes),
        None => (Vec::new(), Vec::new()),
    }
}

/// The same report's runtime **states**, which decide whether a job may start.
fn doctor_runtimes(report_path: Option<&Path>) -> Vec<(String, execution::RuntimeFact)> {
    match doctor_bytes(report_path) {
        Some(bytes) => execution::parse_doctor_runtimes(&bytes),
        None => Vec::new(),
    }
}

fn doctor_bytes(report_path: Option<&Path>) -> Option<Vec<u8>> {
    let path = report_path?;
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() as u64 <= MAX_CACHE_BYTES).then_some(bytes)
}

/// The account label a profile's credential is stored under.
///
/// Same rule as the dialog's `credential_lookup`: an empty `lookup_id` means the
/// default account rather than a missing one.
#[must_use]
pub fn credential_lookup(settings: &AssistSettings) -> &str {
    let stored = settings.credentials.openrouter.lookup_id.as_str();
    if stored.is_empty() {
        "default"
    } else {
        stored
    }
}

/// The credential the `0600` file holds for one account, or why it has none.
///
/// Returns an owned [`Secret`] so the caller's copy is the only one that
/// outlives the call: the store — and with it the store's copy — is dropped and
/// zeroized before this function returns.
///
/// A loose-permission or unparseable file is a **refusal**, not a repair, and
/// it is reported as one. It used to be `.ok()??` — the shape
/// `beat_tracker_update` was found in — which made "the key file is `0644`"
/// and "there is no key" the same answer, and the sentence the user got told
/// them to add the key sitting in front of them (audit A3).
#[must_use]
pub fn openrouter_credential(lookup_id: &str) -> CredentialLookup {
    let Some(path) = files::credentials_path() else {
        return CredentialLookup::Absent;
    };
    let store: CredentialStore = match files::load_credentials(&path) {
        Ok(Some(store)) => store,
        Ok(None) => return CredentialLookup::Absent,
        Err(AssistFileError::Permissions(path, mode)) => {
            return CredentialLookup::Refused {
                path: path.display().to_string(),
                mode: Some(mode),
            };
        }
        Err(_) => {
            return CredentialLookup::Refused {
                path: path.display().to_string(),
                mode: None,
            };
        }
    };
    match store.get("openrouter", lookup_id) {
        // A store that parses but has no entry for this account is an absence,
        // not a refusal: the repair is to add the key, which is exactly what
        // `NoCredential` says.
        None => CredentialLookup::Absent,
        Some(entry) => CredentialLookup::Found(Secret::new(entry.secret.expose().to_string())),
    }
}

/// The credential itself, for the one call site that spawns with it.
#[must_use]
pub fn openrouter_secret(lookup_id: &str) -> Option<Secret> {
    match openrouter_credential(lookup_id) {
        CredentialLookup::Found(secret) => Some(secret),
        _ => None,
    }
}

/// One discovery outcome in the vocabulary the one evaluator reads.
///
/// `OverrideMissing` is audit B6: `Discovery::path()` returns `None` for it, so
/// `--codex-bin` was simply omitted and the helper went hunting for a bare
/// `codex` on an augmented `PATH` — a set-but-missing path silently replaced by
/// a guess, which `settings.rs:196-201` states is exactly what must never
/// happen.
#[must_use]
pub fn codex_fact(discovery: &discover::Discovery) -> execution::CodexFact {
    match &discovery.outcome {
        discover::Outcome::Found { .. } => execution::CodexFact::Found,
        discover::Outcome::NotFound => execution::CodexFact::NotFound,
        discover::Outcome::OverrideMissing { path, reason } => {
            execution::CodexFact::OverrideMissing {
                path: path.display().to_string(),
                reason: reason.clone(),
            }
        }
    }
}

/// Reads `assist.json`, or reports why it could not be read.
///
/// A missing file is defaults; anything else is an error the caller shows. Never
/// a silent reset (§2).
#[must_use]
pub fn load_settings_or_defaults() -> (AssistSettings, Option<String>, Option<PathBuf>) {
    let Some(path) = files::settings_path() else {
        return (
            AssistSettings::default(),
            Some("no per-user configuration directory could be resolved".to_string()),
            None,
        );
    };
    match files::load_settings(&path) {
        Ok(Some(settings)) => (settings, None, Some(path)),
        Ok(None) => (AssistSettings::default(), None, Some(path)),
        Err(error) => (
            AssistSettings::default(),
            Some(error.to_string()),
            Some(path),
        ),
    }
}

/// Everything the caller has that this module cannot read for itself.
#[derive(Clone, Copy, Debug)]
pub struct PlanInputs<'a> {
    /// The workflow itself, not its token.
    ///
    /// Audit A11: this was a `&str` parsed here with
    /// `unwrap_or(WorkflowKind::All)` — the opposite default from
    /// `Boundary::parse`, whose doc calls a defaulted boundary "a silent
    /// widening", and `All` is the workflow that sends audio off the machine.
    /// Both call sites already held a `WorkflowKind`, so the widening is now
    /// unstateable rather than defended against. `Default` goes with it: a
    /// struct default would have to name one workflow, and picking one is the
    /// same mistake in a different place.
    pub kind: WorkflowKind,
    /// Whether an authored lyric sheet is known to this side (chosen, or a
    /// sibling `<stem>.lyrics.txt`). Decides whether `TC-WORDING` is composed.
    pub has_lyric_reference: bool,
    /// True once the user has confirmed this job's data boundary.
    pub boundary_confirmed: bool,
    /// A session credential imported from the environment at startup. The key
    /// itself stays with its owner; this is only its fingerprint.
    pub session_fingerprint: Option<&'a str>,
    /// A doctor report to read runtime identities from, when one was taken.
    pub doctor_report: Option<&'a Path>,
}

/// Resolves and freezes one job's route graph (§5 invariant 3).
///
/// Called **once**, at Start. Nothing re-runs it against current settings
/// afterwards, which is the whole of the invariant: a settings edit mid-job
/// changes the next job and only the next one.
#[must_use]
pub fn resolve(inputs: &PlanInputs<'_>) -> ExecutionPlan {
    let (settings, settings_error, settings_path) = load_settings_or_defaults();
    let kind = inputs.kind;

    let lookup = credential_lookup(&settings).to_string();
    let (stored, credential_source) = match openrouter_credential(&lookup) {
        CredentialLookup::Found(secret) => (Some(secret), CredentialSource::File),
        // A refused file is still a refusal when a session key exists — the
        // session key is the one the job would use, so it is the honest answer
        // and the dialog is where the permission fault gets explained.
        CredentialLookup::Absent | CredentialLookup::Refused { .. }
            if inputs.session_fingerprint.is_some() =>
        {
            (None, CredentialSource::Session)
        }
        CredentialLookup::Refused { path, mode } => {
            (None, CredentialSource::Refused { path, mode })
        }
        CredentialLookup::Absent => (None, CredentialSource::None),
    };
    let fingerprint = match &stored {
        Some(secret) => Some(secret.fingerprint()),
        None => inputs.session_fingerprint.map(str::to_string),
    };
    // The key itself is not kept: `AssistJob::start` reads it again through
    // `openrouter_secret` at the moment it spawns, so a plan value sitting in
    // memory for the length of a confirmation step is not a copy anyone has to
    // reason about (§3, "one owner").
    drop(stored);

    let catalog = catalog_facts();
    let (runtime_versions, model_digests) = doctor_facts(inputs.doctor_report);
    let facts = ExecutionFacts {
        resolved_at_utc: execution::format_rfc3339_utc(now_seconds()),
        credential_present: credential_source.is_usable(),
        credential_fingerprint: fingerprint,
        catalog_revision: catalog.as_ref().map(|(revision, _)| revision.clone()),
        runtime_versions,
        model_digests,
        boundary_confirmed: inputs.boundary_confirmed,
    };
    let snapshot = execution::resolve(&settings, kind, inputs.has_lyric_reference, &facts);
    // Discovery is a login-shell spawn, so it runs only for a graph that
    // actually has a Codex route in it — but it *does* run, which is the other
    // half of audit A1: this gate had no Codex arm at all, and the Routing
    // tab's `Blocked — codex not found` sat next to a Start that was accepted.
    let codex = snapshot
        .contracts
        .iter()
        .any(|entry| entry.route_type == RouteType::Codex)
        .then(|| {
            discover::resolve_cached(
                "codex",
                settings.local_runtimes.codex_bin.as_deref(),
                discover::Options::thorough(),
            )
        });
    let blocks = execution::preflight(
        &snapshot,
        &PreflightFacts {
            credential: credential_source.fact(),
            catalog_models: catalog.map(|(_, models)| models),
            codex: codex
                .as_ref()
                .map_or(execution::CodexFact::Unknown, codex_fact),
            local_runtimes: doctor_runtimes(inputs.doctor_report),
            // The picker's filters are the dialog's; this side has no way to
            // count what they would leave, and inventing a zero would refuse a
            // job for a fact nobody measured.
            eligible_models: None,
        },
    );
    ExecutionPlan {
        snapshot,
        blocks,
        codex,
        credential_source,
        settings_error,
        settings_path,
        credential_lookup: lookup,
        ask_resolved_to_none: execution::stored_ask_contracts(
            &settings,
            kind,
            inputs.has_lyric_reference,
        ),
        local_runtimes: settings.local_runtimes.clone(),
    }
}

/// Writes the snapshot into the job's output directory, before the helper runs.
///
/// One call per job, from `AssistController::start`, and nothing else in the
/// application writes this path — which is what makes §5 invariant 3 checkable
/// from outside: a settings edit mid-run cannot change bytes nobody rewrites.
/// The write is atomic for the ordinary reason, so a crash mid-write leaves the
/// previous job's record rather than a truncated one.
///
/// The output directory is shared across jobs for the same track (the Python
/// caches live there), so a second job for the same track *does* replace this
/// file — with its own resolution, at its own Start. That is the correct
/// behaviour and not a re-resolution of the first: the first job's manifest
/// already embedded its copy.
pub fn write_snapshot(
    output_dir: &Path,
    snapshot: &ExecutionSnapshot,
) -> Result<PathBuf, AssistFileError> {
    std::fs::create_dir_all(output_dir)
        .map_err(|_| AssistFileError::Directory(output_dir.to_path_buf()))?;
    let path = output_dir.join(SNAPSHOT_FILE_NAME);
    let bytes = snapshot
        .to_bytes()
        .map_err(|_| AssistFileError::Write(path.clone()))?;
    crate::process::publish::atomic_write(&path, &bytes)
        .map_err(|_| AssistFileError::Write(path.clone()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::assist::contracts::ContractId;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-assist-plan")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_snapshot_written_into_a_job_folder_is_readable_and_stable() {
        let root = scratch("write");
        let settings = AssistSettings::default();
        let facts = ExecutionFacts {
            resolved_at_utc: "2026-08-05T12:00:00Z".to_string(),
            ..ExecutionFacts::default()
        };
        let snapshot = execution::resolve(&settings, WorkflowKind::Lyrics, true, &facts);
        let path = write_snapshot(&root, &snapshot).expect("write");
        assert_eq!(path.file_name().unwrap(), SNAPSHOT_FILE_NAME);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(ExecutionSnapshot::parse(&bytes).unwrap(), snapshot);
        // Deterministic: the same value writes the same bytes, which is what
        // makes the mid-job negative control a `cmp` rather than a judgement.
        write_snapshot(&root, &snapshot).expect("write");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn the_doctor_report_supplies_runtime_identity_and_an_absent_one_supplies_nothing() {
        let root = scratch("doctor");
        let report = root.join("doctor.json");
        std::fs::write(
            &report,
            br#"{"schema_version":"musializer.doctor/v1","runtimes":{
                "whisper":{"state":"available","version":"whisper.cpp 1.8.6",
                           "model_sha256":"beef"},
                "mms_ctc_aligner":{"state":"available"}}}"#,
        )
        .unwrap();
        let (versions, digests) = doctor_facts(Some(&report));
        assert_eq!(
            versions,
            vec![("whisper".to_string(), "whisper.cpp 1.8.6".to_string())]
        );
        assert_eq!(digests, vec![("whisper".to_string(), "beef".to_string())]);
        assert_eq!(doctor_facts(None), (Vec::new(), Vec::new()));
        assert_eq!(
            doctor_facts(Some(&root.join("absent.json"))),
            (Vec::new(), Vec::new())
        );
    }

    /// The invariant the whole tranche turns on, at the level this module owns:
    /// resolving twice from *different* settings gives different snapshots, and
    /// a snapshot already written is never touched again.
    #[test]
    fn a_written_snapshot_is_not_affected_by_a_later_settings_change() {
        let root = scratch("frozen");
        let facts = ExecutionFacts {
            resolved_at_utc: "2026-08-05T12:00:00Z".to_string(),
            ..ExecutionFacts::default()
        };
        let first = execution::resolve(
            &AssistSettings::default(),
            WorkflowKind::Lyrics,
            true,
            &facts,
        );
        let path = write_snapshot(&root, &first).expect("write");
        let before = std::fs::read(&path).unwrap();

        // The user edits their routing while the job runs.
        let mut edited = AssistSettings::default();
        let mut route = execution::recommended_route(ContractId::Align).unwrap();
        route.model_id = Some("qwen3-fa".to_string());
        route.runtime_id = "qwen3-fa".to_string();
        edited
            .profiles
            .push(musializer_core::assist::settings::Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes: std::collections::BTreeMap::from([(ContractId::Align, route)]),
            });
        edited.active_profile = "studio".to_string();
        let second = execution::resolve(&edited, WorkflowKind::Lyrics, true, &facts);
        assert_ne!(first, second, "the edit really does change the resolution");

        // Nothing rewrote the running job's file.
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
