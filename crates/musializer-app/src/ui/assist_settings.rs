//! The **AI settings** dialog: one modal surface for assist routing, local
//! runtimes, Codex, OpenRouter and the privacy/provenance summary.
//!
//! Design intent: `docs/LYRICS_TIMING_RESEARCH_PLAN.md`, "Visible settings entry
//! point and dialog". Authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` §1 (task
//! contracts), §2 (the preferences record), §3 (credential lifecycle) and §5
//! (fallback rules). Where the two disagree the contracts document wins, and it
//! is the one cited in the code below.
//!
//! **There is no oracle for any of this.** The frozen C has one hard-coded
//! OpenRouter model and reads its key out of a repository `.env`; every control
//! here is new work. So the rules this file follows are stated rather than
//! inherited:
//!
//! - **This is a settings surface only.** Opening it starts no analysis, and
//!   nothing here can change a job in flight (§5 invariant 3). Every routing
//!   change says, in the interface, that it applies to the next job.
//! - **An absent cache is "never fetched", not empty.** A corrupt settings file
//!   is an error state, not a silent reset. A loose-permission credentials file
//!   shows the refusal and the fix. No field is ever invented from missing data,
//!   and catalog strings are display data that never become a path (E6).
//! - **Never a character of a key.** The Replace field draws a fixed bullet per
//!   typed character — see [`masked_field`], whose whole contract is that its
//!   output is a function of the *length* alone, so two different keys of the
//!   same length are pixel-identical. The gate asserts exactly that by comparing
//!   two captures.
//! - **The network is only touched on an explicit press.** `catalog.network_allowed`
//!   is false by default, `Test` and `Refresh` are the only two controls that can
//!   open a socket, and both have an environment stub so a capture never does.
//!
//! # Why a dialog-scoped focus model rather than an app-wide one
//!
//! `ui/widgets.rs` has no focus or traversal model at all (recorded as a
//! limitation in `FEATURE_PARITY_PLAN.md`, LT1-R R9). A modal is a *closed*
//! focus scope, which is the one place a traversal ring can be complete without
//! being half a model: every control that can take focus is drawn by this file,
//! in an order this file computes ([`AssistSettingsDialog::focus_order`]). Tab
//! and Shift-Tab walk it, Enter activates, Escape closes. Nothing is retrofitted
//! onto the shell.
//!
//! # Why the dialog owns a second [`Widgets`] bank
//!
//! Input blocking falls out of the oracle's own claim rule rather than needing a
//! new mechanism: a press is claimed by the *first* widget to see it
//! (`ui_widgets.c:139-160`), and one `active_button_id` holds that claim for the
//! whole frame. So the shell draws a full-window blocker into **its** bank before
//! any panel, and the dialog draws its own controls into a bank of its own, which
//! the blocker cannot reach. The shell's keyboard is skipped in the same branch,
//! and the timeline strip is not drawn at all while the modal is up — the lyrics
//! cue field reads raylib's keyboard directly rather than through the widget bank
//! (`ui/text_input.rs`), so leaving it drawn would let a key typed into the
//! Replace field also land in a cue, which is a credential in a `.musi` file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use musializer_core::assist::contracts::{
    Boundary, ContractId, FallbackPolicy, RouteType, ALL_CONTRACTS,
};
use musializer_core::assist::models_dir::ModelsDirResolution;
use musializer_core::assist::secret::Secret;
use musializer_core::assist::settings::{
    AssistSettings, CredentialMode, Profile, Provider, ReasoningEffort, Route, StemSeparation,
    SCHEMA as SETTINGS_SCHEMA,
};
use musializer_core::assist::suitability::{self, Suitability};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::assist::files::{self, AssistFileError};
use musializer_runtime::assist::models;
use musializer_runtime::font::Faces;
use raylib::prelude::{Color, RaylibDraw, RaylibDrawHandle, Vector2};
use serde::Deserialize;

use super::scale::UiScale;
use super::theme::{color, metric};
use super::widgets::{self, ButtonStyle, Widgets};

// ---------------------------------------------------------------------------
// Probe seams
// ---------------------------------------------------------------------------
//
// Environment variables rather than `--ui-probe` keys, for the reason
// `panels/assist.rs` records for `MUSIALIZER_ASSIST_PROBE_DIR`: the probe
// grammar lives in `cli.rs`, which this tranche does not own. Nothing outside a
// capture reads any of them, and each one is inert when unset.

/// Opens the dialog at a section: `routing`, `local`, `codex`, `openrouter`,
/// `privacy`, or `1` for the first section.
pub const PROBE_OPEN_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_OPEN";

/// How many Tab steps to apply once the dialog is open, so a focus ring is
/// photographable. A headless run has no keyboard any more than it has a
/// pointer.
pub const PROBE_TAB_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_TAB";

/// Scroll offset in logical pixels, for photographing the bottom of a section.
pub const PROBE_SCROLL_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_SCROLL";

/// A **fixture** credential to seed the masked Replace field with.
///
/// The point of the variable is to prove that nothing of its value reaches a
/// pixel or a report line, which is why the gate plants a sentinel here and then
/// greps for it. Never a real key: the dialog's own Replace flow is how a user
/// enters one.
pub const PROBE_KEY_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_KEY";

/// Pins "now" to an RFC 3339 instant, so a cache age badge is the same in every
/// run of the same capture.
pub const PROBE_NOW_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_NOW";

/// Stubs the `Test` outcome. **Set in every capture that can reach the
/// OpenRouter section**, so no gate run can open a socket.
pub const PROBE_KEY_TEST_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_KEY_TEST";

/// Stubs the catalog `Refresh` outcome, for the same reason.
pub const PROBE_REFRESH_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_REFRESH";

/// A doctor report read from a file instead of by running `musializer_doctor.py`,
/// so the Local models section is photographable without a Python round trip.
pub const PROBE_DOCTOR_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_DOCTOR";

/// Presses Escape once at open, so "Escape closes" is assertable from a capture
/// run's report rather than only from a unit test.
pub const PROBE_ESCAPE_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_ESCAPE";

/// Marks a route override as edited at open, so the unsaved-changes confirm step
/// is reachable from a capture.
pub const PROBE_DIRTY_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_DIRTY";

/// Presses Enter once on the focused control. Without it, "Enter activates" is
/// only ever a unit test — nothing in a headless run can press a key, so the
/// whole write path (Save, Forget, Test) would be unreachable from the gate.
pub const PROBE_ACTIVATE_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_ACTIVATE";

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// The five sections of the dialog, in the order the design document lists them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    #[default]
    Routing,
    LocalModels,
    Codex,
    OpenRouter,
    Privacy,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Routing,
        Section::LocalModels,
        Section::Codex,
        Section::OpenRouter,
        Section::Privacy,
    ];

    /// The stable token, used by the probe seam and the report line.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Section::Routing => "routing",
            Section::LocalModels => "local",
            Section::Codex => "codex",
            Section::OpenRouter => "openrouter",
            Section::Privacy => "privacy",
        }
    }

    /// The rail label.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Section::Routing => "Routing",
            Section::LocalModels => "Local models",
            Section::Codex => "Codex",
            Section::OpenRouter => "OpenRouter",
            Section::Privacy => "Privacy",
        }
    }

    /// The heading drawn at the top of the section body.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Section::Routing => "ROUTING",
            Section::LocalModels => "LOCAL MODELS",
            Section::Codex => "CODEX",
            Section::OpenRouter => "OPENROUTER",
            Section::Privacy => "PRIVACY AND DIAGNOSTICS",
        }
    }

    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "1" | "routing" => Some(Section::Routing),
            "local" | "local-models" => Some(Section::LocalModels),
            "codex" => Some(Section::Codex),
            "openrouter" => Some(Section::OpenRouter),
            "privacy" | "diagnostics" => Some(Section::Privacy),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Time, and the honest age badge
// ---------------------------------------------------------------------------

/// Seconds since the Unix epoch for an RFC 3339 timestamp in `Z`.
///
/// Both discovery caches write `time.strftime("%Y-%m-%dT%H:%M:%SZ", gmtime())`,
/// so exactly that shape is what has to parse. Anything else answers `None`
/// rather than a guessed instant — an age computed from a misread timestamp is
/// precisely the invented field the honesty rule forbids.
#[must_use]
pub fn parse_rfc3339_utc(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> { text[from..to].parse::<i64>().ok() };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's `days_from_civil`. Written out rather than pulled in as a
/// dependency: it is nine lines and this is the only date arithmetic in the
/// application.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// What a cache's age means for the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheBadge {
    /// No cache file at all. **Not** an empty catalog.
    NeverFetched,
    /// A file exists but could not be read as this schema.
    Unreadable,
    /// Fetched recently enough to be treated as current.
    Fresh,
    /// Older than [`STALE_AFTER_SECONDS`], or carrying a timestamp this build
    /// cannot parse.
    Stale,
}

impl CacheBadge {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            CacheBadge::NeverFetched => "never fetched",
            CacheBadge::Unreadable => "unreadable",
            CacheBadge::Fresh => "current",
            CacheBadge::Stale => "stale",
        }
    }

    fn tint(self) -> Color {
        match self {
            CacheBadge::Fresh => color::ui_success(),
            CacheBadge::Stale => color::ui_warning(),
            CacheBadge::NeverFetched => color::ui_muted(),
            CacheBadge::Unreadable => color::ui_danger(),
        }
    }
}

/// A catalog older than this reads as stale. Seven days: OpenRouter's list moves
/// in weeks rather than hours, and a badge that goes amber every afternoon is a
/// badge nobody reads.
pub const STALE_AFTER_SECONDS: i64 = 7 * 86_400;

/// A human age for a fetch timestamp, and whether it is stale.
///
/// `None` for the timestamp is "never fetched"; an unparsable one is stale with
/// the raw string echoed, because showing what the file actually says beats
/// showing a computed age derived from a misreading.
#[must_use]
pub fn age_badge(fetched_at_utc: Option<&str>, now: Option<i64>) -> (CacheBadge, String) {
    let Some(stamp) = fetched_at_utc else {
        return (CacheBadge::NeverFetched, "never fetched".to_string());
    };
    let (Some(then), Some(now)) = (parse_rfc3339_utc(stamp), now) else {
        return (CacheBadge::Stale, format!("fetched {stamp} (age unknown)"));
    };
    let age = now - then;
    if age < 0 {
        // A cache stamped in the future is a clock the application cannot
        // reconcile; saying so beats printing a negative age.
        return (CacheBadge::Stale, format!("fetched {stamp} (clock skew)"));
    }
    let described = if age < 90 {
        "just now".to_string()
    } else if age < 5400 {
        format!("{} min ago", age / 60)
    } else if age < 172_800 {
        format!("{} h ago", age / 3600)
    } else {
        format!("{} days ago", age / 86_400)
    };
    let badge = if age >= STALE_AFTER_SECONDS {
        CacheBadge::Stale
    } else {
        CacheBadge::Fresh
    };
    (badge, described)
}

/// "Now", pinned by [`PROBE_NOW_VARIABLE`] when a capture is running.
fn now_utc() -> Option<i64> {
    if let Ok(pinned) = std::env::var(PROBE_NOW_VARIABLE) {
        return parse_rfc3339_utc(pinned.trim());
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
}

// ---------------------------------------------------------------------------
// Masking
// ---------------------------------------------------------------------------

/// The longest run of bullets the Replace field will draw.
///
/// Capped so the drawn width does not report the key's length past a coarse
/// bound (§3: "never any character of the key, never a length"). Past the cap
/// the field draws the cap plus a `+`, which says "longer than this" without
/// saying how much longer.
pub const MASK_CAP: usize = 24;

/// What the Replace field draws for a credential of `length` characters.
///
/// **A function of the length alone.** That is the whole contract, and it is what
/// makes the gate's assertion possible: two different keys of the same length
/// produce byte-identical strings, so the two crops are pixel-identical. A field
/// that leaked one character would break that immediately.
#[must_use]
pub fn masked_field(length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    let drawn = length.min(MASK_CAP);
    let mut text = "\u{2022}".repeat(drawn);
    if length > MASK_CAP {
        text.push('+');
    }
    text
}

// ---------------------------------------------------------------------------
// Loaded state
// ---------------------------------------------------------------------------

/// What became of the settings file (§2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsSource {
    /// No file yet. The only case that legitimately means "use the defaults".
    Absent,
    Loaded,
    /// A corrupt, oversized or foreign-schema file. **Never a silent reset**: the
    /// dialog refuses to save over it until the user says so.
    Error(String),
    /// No per-user configuration directory could be resolved at all.
    NoPath,
}

impl SettingsSource {
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            SettingsSource::Absent => "absent (defaults)".to_string(),
            SettingsSource::Loaded => "loaded".to_string(),
            SettingsSource::Error(reason) => format!("error: {reason}"),
            SettingsSource::NoPath => "no config directory".to_string(),
        }
    }
}

/// What is known about the OpenRouter credential without holding one (§3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialState {
    /// No credentials file, and nothing imported this session.
    None,
    /// Held in this process only, from `OPENROUTER_API_KEY` at startup, or from
    /// a Replace the user chose not to persist.
    Session { fingerprint: String },
    /// Present in the `0600` file.
    File {
        fingerprint: String,
        label: Option<String>,
    },
    /// Refused, not repaired (§3, "read refusal"). The mode is shown because the
    /// fix is `chmod 600` and the user needs to know what it is now.
    Refused { path: String, mode: u32 },
    /// Any other read failure.
    Error(String),
}

impl CredentialState {
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            CredentialState::None => "none".to_string(),
            CredentialState::Session { fingerprint } => format!("session({fingerprint})"),
            CredentialState::File { fingerprint, .. } => format!("file({fingerprint})"),
            CredentialState::Refused { mode, .. } => format!("refused({mode:04o})"),
            CredentialState::Error(reason) => format!("error: {reason}"),
        }
    }

    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(
            self,
            CredentialState::Session { .. } | CredentialState::File { .. }
        )
    }
}

/// One normalized OpenRouter catalog entry (`tools/provider_catalog.py`).
///
/// Every field is display data. E6: a catalog string never becomes a path or a
/// shell fragment, and nothing in this file joins one onto anything.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    #[serde(default)]
    pub output_modalities: Vec<String>,
    #[serde(default)]
    pub context_length: Option<f64>,
    #[serde(default)]
    pub pricing: CatalogPricing,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CatalogPricing {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub audio: Option<String>,
}

/// The OpenRouter catalog cache document.
#[derive(Clone, Debug, Deserialize)]
pub struct CatalogCache {
    pub schema_version: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub fetched_at_utc: Option<String>,
    #[serde(default)]
    pub filters: BTreeMap<String, String>,
    #[serde(default)]
    pub models: Vec<CatalogModel>,
}

/// The Codex catalog cache document (`tools/codex_model_discovery.py`).
#[derive(Clone, Debug, Deserialize)]
pub struct CodexCache {
    pub schema_version: String,
    #[serde(default)]
    pub fetched_at_utc: Option<String>,
    #[serde(default)]
    pub models: Vec<CodexModel>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CodexModel {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<CodexEffort>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CodexEffort {
    #[serde(default)]
    pub reasoning_effort: String,
}

/// The label a Codex route shows when discovery never ran or failed.
///
/// §5 rule 6: "Codex discovery failure preserves `Codex default`. Never a
/// guessed model id." The string is exactly the one
/// `tools/codex_model_discovery.py` uses.
pub const CODEX_DEFAULT_LABEL: &str = "Codex default";

/// A cache slot: absent, unreadable, or a document.
#[derive(Clone, Debug)]
pub enum CacheSlot<T> {
    NeverFetched,
    Unreadable(String),
    Loaded(T),
}

impl<T> CacheSlot<T> {
    #[must_use]
    pub fn badge(&self, fetched_at_utc: Option<&str>, now: Option<i64>) -> (CacheBadge, String) {
        match self {
            CacheSlot::NeverFetched => (CacheBadge::NeverFetched, "never fetched".to_string()),
            CacheSlot::Unreadable(reason) => (CacheBadge::Unreadable, reason.clone()),
            CacheSlot::Loaded(_) => age_badge(fetched_at_utc, now),
        }
    }

    #[must_use]
    pub fn document(&self) -> Option<&T> {
        match self {
            CacheSlot::Loaded(document) => Some(document),
            _ => None,
        }
    }
}

/// One runtime's identity, as `tools/runtime_inventory.py` reports it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RuntimeIdentity {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub model_path: Option<String>,
    #[serde(default)]
    pub model_sha256: Option<String>,
    #[serde(default)]
    pub language_support: Option<String>,
    #[serde(default)]
    pub gpu_ready: Option<bool>,
    #[serde(default)]
    pub remediation: Option<String>,
}

/// The subset of a doctor report this dialog reads.
#[derive(Clone, Debug, Deserialize)]
pub struct DoctorReport {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub runtimes: BTreeMap<String, RuntimeIdentity>,
}

/// The three runtimes the Local models section names, in display order.
pub const RUNTIME_ROWS: [(&str, &str); 3] = [
    ("whisper", "Whisper"),
    ("mms_ctc_aligner", "MMS/CTC forced aligner"),
    ("stem_separator", "Stem separator"),
];

// ---------------------------------------------------------------------------
// The recommended profile, and route resolution
// ---------------------------------------------------------------------------

/// Whether a route is ready to run, and why not when it is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Blocked(String),
    /// Nothing has measured this yet. Distinct from `Blocked`, because "we have
    /// not looked" and "we looked and it is missing" are different sentences and
    /// the second one is a lie when only the first is true.
    Unknown(String),
}

impl Readiness {
    #[must_use]
    pub fn token(&self) -> &str {
        match self {
            Readiness::Ready => "ready",
            Readiness::Blocked(_) => "blocked",
            Readiness::Unknown(_) => "unknown",
        }
    }

    fn tint(&self) -> Color {
        match self {
            Readiness::Ready => color::ui_success(),
            Readiness::Blocked(_) => color::ui_danger(),
            Readiness::Unknown(_) => color::ui_muted(),
        }
    }

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Readiness::Ready => "Ready".to_string(),
            Readiness::Blocked(reason) => reason.clone(),
            Readiness::Unknown(reason) => reason.clone(),
        }
    }
}

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
    /// `Codex default` — the documented fallback, not a guess (§5 rule 6).
    #[must_use]
    pub fn model_label(&self) -> String {
        let Some(route) = &self.route else {
            return "\u{2014}".to_string();
        };
        match (&route.model_id, route.route_type) {
            (Some(id), _) => id.clone(),
            (None, RouteType::Codex) => CODEX_DEFAULT_LABEL.to_string(),
            (None, RouteType::Builtin) => route.runtime_id.clone(),
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

/// The user profile overrides are written into. `recommended` cannot be stored
/// (§2), so the first override creates this one.
pub const OVERRIDE_PROFILE_ID: &str = "custom";
pub const OVERRIDE_PROFILE_LABEL: &str = "Custom";

/// Stores a per-task override, creating the `custom` profile on first use and
/// making it active.
///
/// Removing an override that equals the recommended route again drops the entry,
/// so a user who cycles back to the default gets a file that says so rather than
/// one that pins the current default forever.
pub fn set_override(settings: &mut AssistSettings, route: Route) {
    let contract = route.contract;
    let matches_recommended = recommended_route(contract).is_some_and(|base| base == route);
    if settings.profile(OVERRIDE_PROFILE_ID).is_none() {
        if matches_recommended {
            return;
        }
        settings.profiles.push(Profile {
            id: OVERRIDE_PROFILE_ID.to_string(),
            label: OVERRIDE_PROFILE_LABEL.to_string(),
            routes: BTreeMap::new(),
        });
    }
    settings.active_profile = OVERRIDE_PROFILE_ID.to_string();
    let Some(profile) = settings
        .profiles
        .iter_mut()
        .find(|profile| profile.id == OVERRIDE_PROFILE_ID)
    else {
        return;
    };
    if matches_recommended {
        profile.routes.remove(&contract);
    } else {
        profile.routes.insert(contract, route);
    }
}

// ---------------------------------------------------------------------------
// Pickers
// ---------------------------------------------------------------------------

/// The runtime ids a `local-proc` route may name for a contract.
///
/// A short in-repo list rather than a probe: these are the lanes
/// `tools/external_analysis.py` actually knows how to run, and offering a
/// runtime the helper cannot invoke would be a picker inventing a capability.
#[must_use]
pub fn local_runtimes_for(contract: ContractId) -> &'static [&'static str] {
    match contract {
        ContractId::Coarse | ContractId::Verify => &["whisper.cpp"],
        ContractId::Align => &["mms-ctc", "qwen3-fa"],
        _ => &[],
    }
}

/// Whether a catalog model's modalities can serve a contract at all.
///
/// The first eligibility filter, and explicitly **not sufficient** (§1): a model
/// that accepts audio may still be wrong for the task, which is what the
/// suitability overlay is for.
#[must_use]
pub fn modalities_fit(contract: ContractId, model: &CatalogModel) -> bool {
    let takes_audio = model
        .input_modalities
        .iter()
        .any(|modality| modality == "audio");
    let emits_text = model.output_modalities.is_empty()
        || model
            .output_modalities
            .iter()
            .any(|modality| modality == "text");
    match contract {
        ContractId::Coarse | ContractId::Semantic | ContractId::Verify => takes_audio && emits_text,
        ContractId::Wording | ContractId::Plan => emits_text,
        ContractId::Measured | ContractId::Align => false,
    }
}

/// The model ids a picker offers for one contract and route type.
///
/// Filtered by contract eligibility **and** the suitability overlay:
/// `unsupported` is never offered however the picker is configured, and
/// `experimental` needs `catalog.show_experimental` (§1 and
/// `assist::suitability`). The currently selected id is always included even
/// when it would otherwise be filtered out, because a picker that silently drops
/// the value it is showing cannot be used to change it.
#[must_use]
pub fn model_options(
    contract: ContractId,
    route_type: RouteType,
    catalog: Option<&CatalogCache>,
    codex: Option<&CodexCache>,
    show_experimental: bool,
    selected: Option<&str>,
) -> Vec<String> {
    let mut options: Vec<String> = match route_type {
        RouteType::Builtin => Vec::new(),
        RouteType::LocalProc => local_runtimes_for(contract)
            .iter()
            .map(|id| (*id).to_string())
            .collect(),
        RouteType::Codex => {
            let mut ids = vec![CODEX_DEFAULT_LABEL.to_string()];
            if let Some(cache) = codex {
                ids.extend(
                    cache
                        .models
                        .iter()
                        .filter(|model| !model.hidden)
                        .map(|model| model.id.clone()),
                );
            }
            return ids;
        }
        RouteType::OpenRouter => catalog.map_or_else(Vec::new, |cache| {
            cache
                .models
                .iter()
                .filter(|model| modalities_fit(contract, model))
                .map(|model| model.id.clone())
                .collect()
        }),
    };
    options.retain(|id| suitability::status(id, contract).offered(show_experimental));
    if let Some(selected) = selected {
        if !selected.is_empty() && !options.iter().any(|id| id == selected) {
            options.insert(0, selected.to_string());
        }
    }
    options
}

/// The next entry in a cycling picker. Returns the first option when the current
/// value is absent, and `None` when there is nothing to offer.
#[must_use]
pub fn cycle<T: PartialEq + Clone>(options: &[T], current: Option<&T>) -> Option<T> {
    if options.is_empty() {
        return None;
    }
    let index = current
        .and_then(|value| options.iter().position(|option| option == value))
        .map_or(0, |index| (index + 1) % options.len());
    options.get(index).cloned()
}

// ---------------------------------------------------------------------------
// Key test
// ---------------------------------------------------------------------------

/// The six outcomes §3 names. Each one is a distinct actionable state, never a
/// generic failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyTest {
    Ok {
        label: Option<String>,
        free_tier: Option<bool>,
    },
    Invalid,
    Revoked,
    RateLimited,
    NoNetwork(String),
    NoKey,
}

impl KeyTest {
    #[must_use]
    pub fn token(&self) -> &'static str {
        match self {
            KeyTest::Ok { .. } => "ok",
            KeyTest::Invalid => "invalid",
            KeyTest::Revoked => "revoked",
            KeyTest::RateLimited => "rate-limited",
            KeyTest::NoNetwork(_) => "no-network",
            KeyTest::NoKey => "no-key",
        }
    }

    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        match token.trim() {
            "ok" => Some(KeyTest::Ok {
                label: Some("stub".to_string()),
                free_tier: Some(false),
            }),
            "invalid" => Some(KeyTest::Invalid),
            "revoked" => Some(KeyTest::Revoked),
            "rate-limited" => Some(KeyTest::RateLimited),
            "no-network" => Some(KeyTest::NoNetwork("stubbed".to_string())),
            "no-key" => Some(KeyTest::NoKey),
            _ => None,
        }
    }

    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            KeyTest::Ok { label, free_tier } => {
                let mut text = "Key accepted".to_string();
                if let Some(label) = label {
                    text.push_str(&format!(" \u{00b7} {label}"));
                }
                if let Some(true) = free_tier {
                    text.push_str(" \u{00b7} free tier");
                }
                text
            }
            KeyTest::Invalid => "OpenRouter rejected this key as invalid.".to_string(),
            KeyTest::Revoked => "OpenRouter reports this key as revoked or disabled.".to_string(),
            KeyTest::RateLimited => {
                "OpenRouter rate-limited the check. Wait and test again.".to_string()
            }
            KeyTest::NoNetwork(detail) => format!("Could not reach OpenRouter: {detail}"),
            KeyTest::NoKey => "There is no key to test.".to_string(),
        }
    }

    fn tint(&self) -> Color {
        match self {
            KeyTest::Ok { .. } => color::ui_success(),
            KeyTest::Invalid | KeyTest::Revoked => color::ui_danger(),
            KeyTest::RateLimited | KeyTest::NoNetwork(_) => color::ui_warning(),
            KeyTest::NoKey => color::ui_muted(),
        }
    }
}

/// `GET https://openrouter.ai/api/v1/key` — read-only, non-inference, spends no
/// credits (§3).
pub const KEY_TEST_URL: &str = "https://openrouter.ai/api/v1/key";

/// Runs the key check, off the frame thread.
///
/// The request goes out through `curl` reading its configuration from **stdin**,
/// which is the only way to reach a socket from here without a new dependency
/// *and* keep E2: no flag ever takes a key, so `/proc/<pid>/cmdline` never has
/// one, and nothing is written to disk (which would be the durable plaintext
/// copy §3 forbids). The secret is moved in and moved back out, so there is
/// exactly one owner throughout.
fn run_key_test(secret: Secret) -> (KeyTest, Secret) {
    let mut child = match Command::new("curl")
        .arg("--config")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return (
                KeyTest::NoNetwork(format!(
                    "curl could not be started ({error}); this build has no built-in HTTP client"
                )),
                secret,
            )
        }
    };
    {
        use std::io::Write;
        let Some(stdin) = child.stdin.as_mut() else {
            let _ = child.kill();
            return (
                KeyTest::NoNetwork("curl accepted no configuration".to_string()),
                secret,
            );
        };
        // `--config` takes `key = "value"` lines. The URL and the header both go
        // here so neither reaches argv.
        let configuration = format!(
            "url = \"{KEY_TEST_URL}\"\nheader = \"Authorization: Bearer {}\"\nsilent\nshow-error\nmax-time = 15\nwrite-out = \"\\n%{{http_code}}\"\n",
            secret.expose()
        );
        if stdin.write_all(configuration.as_bytes()).is_err() {
            let _ = child.kill();
            return (
                KeyTest::NoNetwork("curl closed its input".to_string()),
                secret,
            );
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return (KeyTest::NoNetwork(error.to_string()), secret),
    };
    let body = String::from_utf8_lossy(&output.stdout);
    let status = body.rsplit('\n').next().unwrap_or("").trim().to_string();
    let document = body
        .rsplit_once('\n')
        .map_or("", |(head, _)| head)
        .to_string();
    let outcome = match status.as_str() {
        "200" => {
            // E10: on any failure the report names status, provider and
            // contract — never the request. Here the *success* path reads only
            // two display fields out of the response.
            let parsed: Option<serde_json::Value> = serde_json::from_str(&document).ok();
            let data = parsed.as_ref().and_then(|value| value.get("data"));
            KeyTest::Ok {
                label: data
                    .and_then(|data| data.get("label"))
                    .and_then(|label| label.as_str())
                    .map(str::to_string),
                free_tier: data
                    .and_then(|data| data.get("is_free_tier"))
                    .and_then(serde_json::Value::as_bool),
            }
        }
        "401" => KeyTest::Invalid,
        "403" => KeyTest::Revoked,
        "429" => KeyTest::RateLimited,
        "" => KeyTest::NoNetwork(
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("curl reported no status")
                .to_string(),
        ),
        other => KeyTest::NoNetwork(format!("OpenRouter answered HTTP {other}")),
    };
    (outcome, secret)
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// The dialog's geometry for one window size.
///
/// Raylib-free and unit-tested, for the reason `core::ui::transport_bar` is: a
/// layout error is invisible in a capture because everything moves together and
/// still looks self-coherent, so the properties that matter — the content region
/// stays inside the dialog, the rail never eats the body, every rung is
/// monotonic in width — have to be assertable without a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogLayout {
    pub scrim: UiRect,
    pub dialog: UiRect,
    pub header: UiRect,
    pub close: UiRect,
    pub save: UiRect,
    pub tabs: [UiRect; 5],
    pub content: UiRect,
    pub footer: UiRect,
    /// True when the section list is a left rail; false when it is a top row of
    /// tabs, which is what a narrow window gets.
    pub rail_on_left: bool,
}

/// Chrome heights. Constants rather than magic numbers because the content
/// region is derived from them and a test asserts the derivation.
const HEADER_HEIGHT: f32 = 46.0;
const FOOTER_HEIGHT: f32 = 34.0;
const TAB_ROW_HEIGHT: f32 = 34.0;
const RAIL_WIDTH: f32 = 168.0;
const DIALOG_PADDING: f32 = 14.0;
const MAX_DIALOG_WIDTH: f32 = 1040.0;
const MAX_DIALOG_HEIGHT: f32 = 760.0;

/// Below this dialog width the section list moves from a left rail to a top row.
pub const RAIL_MINIMUM_WIDTH: f32 = 790.0;

/// Below this **dialog** width the routing matrix stacks each contract onto two
/// lines instead of six columns.
///
/// A dialog width rather than the body width it is really about, and that is the
/// whole point. Shedding the rail *widens* the body by 168 px, so the body width
/// is not monotonic in the window width — deciding from it would make the matrix
/// collapse at 900 px, come back at 810, and collapse again at 730. That is
/// exactly the vanish-and-reappear defect `core::ui::transport_bar` was rebuilt
/// to make unstateable, and the property to assert is monotonicity, not a
/// threshold. 882 is where a rail-mode body reaches the 676 px the six columns
/// need; below it every layout stacks, rail or tabs.
pub const ROUTING_MATRIX_DIALOG_WIDTH: f32 = 882.0;

/// The narrowest body the six-column matrix fits in: the five fixed columns, the
/// five gaps, and the 90 px floor under the flexible MODEL column. Read by the
/// test that ties the two constants together — the dialog threshold above is
/// only correct *because* it produces at least this body.
#[cfg(test)]
pub const ROUTING_MATRIX_BODY_WIDTH: f32 = 676.0;

impl DialogLayout {
    #[must_use]
    pub fn of(window: (f32, f32)) -> Self {
        let margin = if window.0 >= 1100.0 { 32.0 } else { 12.0 };
        let width = (window.0 - margin * 2.0).clamp(0.0, MAX_DIALOG_WIDTH);
        let height = (window.1 - margin * 2.0).clamp(0.0, MAX_DIALOG_HEIGHT);
        let dialog = UiRect::new(
            ((window.0 - width) * 0.5).max(0.0).round(),
            ((window.1 - height) * 0.5).max(0.0).round(),
            width,
            height,
        );
        let header = UiRect::new(dialog.x, dialog.y, dialog.width, HEADER_HEIGHT.min(height));
        let close = UiRect::new(
            dialog.x + dialog.width - DIALOG_PADDING - 32.0,
            dialog.y + 7.0,
            32.0,
            32.0,
        );
        let save = UiRect::new(close.x - 8.0 - 96.0, dialog.y + 7.0, 96.0, 32.0);

        let rail_on_left = dialog.width >= RAIL_MINIMUM_WIDTH;
        let body_top = dialog.y + header.height;
        let body_height = (dialog.y + dialog.height - body_top - FOOTER_HEIGHT).max(0.0);

        let mut tabs = [UiRect::new(0.0, 0.0, 0.0, 0.0); 5];
        let content;
        if rail_on_left {
            for (index, tab) in tabs.iter_mut().enumerate() {
                *tab = UiRect::new(
                    dialog.x + DIALOG_PADDING,
                    body_top + DIALOG_PADDING + index as f32 * 38.0,
                    RAIL_WIDTH - DIALOG_PADDING,
                    32.0,
                );
            }
            let left = dialog.x + RAIL_WIDTH;
            content = UiRect::new(
                left,
                body_top,
                (dialog.x + dialog.width - left).max(0.0),
                body_height,
            );
        } else {
            let usable = (dialog.width - DIALOG_PADDING * 2.0).max(0.0);
            let cell = (usable - 4.0 * 4.0) / 5.0;
            for (index, tab) in tabs.iter_mut().enumerate() {
                *tab = UiRect::new(
                    dialog.x + DIALOG_PADDING + index as f32 * (cell + 4.0),
                    body_top + 6.0,
                    cell.max(0.0),
                    TAB_ROW_HEIGHT - 4.0,
                );
            }
            let top = body_top + TAB_ROW_HEIGHT + 6.0;
            content = UiRect::new(
                dialog.x,
                top,
                dialog.width,
                (dialog.y + dialog.height - top - FOOTER_HEIGHT).max(0.0),
            );
        }
        let footer = UiRect::new(
            dialog.x,
            dialog.y + dialog.height - FOOTER_HEIGHT,
            dialog.width,
            FOOTER_HEIGHT.min(dialog.height),
        );
        Self {
            scrim: UiRect::new(0.0, 0.0, window.0, window.1),
            dialog,
            header,
            close,
            save,
            tabs,
            content,
            footer,
            rail_on_left,
        }
    }

    /// The region section bodies draw into, inset from [`Self::content`].
    #[must_use]
    pub fn body(&self) -> UiRect {
        UiRect::new(
            self.content.x + DIALOG_PADDING,
            self.content.y + DIALOG_PADDING * 0.5,
            (self.content.width - DIALOG_PADDING * 2.0 - 10.0).max(0.0),
            (self.content.height - DIALOG_PADDING).max(0.0),
        )
    }

    /// Whether the routing matrix has to stack. See
    /// [`ROUTING_MATRIX_DIALOG_WIDTH`] for why this is not measured against the
    /// body it describes.
    #[must_use]
    pub fn routing_compact(&self) -> bool {
        self.dialog.width < ROUTING_MATRIX_DIALOG_WIDTH
    }
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// A focusable control, identified stably enough that Tab order survives a
/// redraw.
///
/// A `u32` rather than an enum with a variant per control: the routing matrix
/// alone has four controls per contract, and the ring has to be built from the
/// live model (a contract with no eligible models has no model picker to focus).
/// The constants below carve the space so two sections cannot collide.
pub type ControlId = u32;

const FOCUS_TAB_BASE: ControlId = 10;
const FOCUS_SAVE: ControlId = 1;
const FOCUS_CLOSE: ControlId = 2;
const FOCUS_ROUTE_BASE: ControlId = 100;
const FOCUS_MODEL_BASE: ControlId = 200;
const FOCUS_FALLBACK_BASE: ControlId = 300;
const FOCUS_ROUTING_EXPERIMENTAL: ControlId = 400;
const FOCUS_ROUTING_PROFILE: ControlId = 401;
const FOCUS_MODELS_DIR: ControlId = 500;
const FOCUS_MODELS_DOCTOR: ControlId = 501;
const FOCUS_MODELS_GPU: ControlId = 502;
const FOCUS_MODELS_STEMS: ControlId = 503;
const FOCUS_CODEX_EFFORT_BASE: ControlId = 600;
const FOCUS_CODEX_REFRESH: ControlId = 610;
const FOCUS_KEY_FIELD: ControlId = 700;
const FOCUS_KEY_TEST: ControlId = 701;
const FOCUS_KEY_REPLACE: ControlId = 702;
const FOCUS_KEY_FORGET: ControlId = 703;
const FOCUS_KEY_SAVE_UNTESTED: ControlId = 704;
const FOCUS_CATALOG_REFRESH: ControlId = 710;
const FOCUS_CATALOG_NETWORK: ControlId = 711;
const FOCUS_CATALOG_REFRESH_ON_OPEN: ControlId = 712;
const FOCUS_PRIVACY_ZDR_BASE: ControlId = 800;
const FOCUS_PRIVACY_COPY: ControlId = 810;

/// The contracts a Codex reasoning effort applies to: the two `codex`-eligible
/// contracts in §1's table.
const CODEX_ELIGIBLE: [ContractId; 2] = [ContractId::Wording, ContractId::Plan];

// ---------------------------------------------------------------------------
// Background work
// ---------------------------------------------------------------------------

/// A job running off the frame thread, so a network round trip or a Python
/// process never holds a `BeginDrawing`/`EndDrawing` pair open.
enum Background {
    KeyTest(Receiver<(KeyTest, Secret)>),
    Refresh(Receiver<Result<(), String>>),
    Doctor(Receiver<Result<DoctorReport, String>>),
}

impl Background {
    fn label(&self) -> &'static str {
        match self {
            Background::KeyTest(_) => "testing the key",
            Background::Refresh(_) => "refreshing the catalog",
            Background::Doctor(_) => "running the doctor",
        }
    }
}

// ---------------------------------------------------------------------------
// The dialog
// ---------------------------------------------------------------------------

/// The AI settings modal.
///
/// Held on [`crate::ui::shell::Shell`] because it outlives a frame. Everything it
/// reads from disk is read once, in [`Self::open`]: a settings surface that
/// `stat`ed four files per frame from inside a drawing pair would be the
/// per-frame syscall `panels/assist.rs` already refuses for the helper probe.
pub struct AssistSettingsDialog {
    open: bool,
    section: Section,
    /// The dialog's own widget bank. See the module comment for why it is not the
    /// shell's.
    widgets: Widgets,

    /// What is on disk.
    saved: AssistSettings,
    /// What the user has edited. `dirty` is `draft != saved`.
    draft: AssistSettings,
    settings_path: Option<PathBuf>,
    settings_source: SettingsSource,

    credentials_path: Option<PathBuf>,
    credentials: CredentialState,
    /// True when a credential was imported from the environment this session.
    session_credential: Option<String>,

    models_dir: Option<ModelsDirResolution>,
    models_dir_error: Option<String>,
    /// The override being typed, mirrored into the draft on every change.
    models_dir_field: String,
    models_dir_focused: bool,

    catalog: CacheSlot<CatalogCache>,
    codex: CacheSlot<CodexCache>,
    codex_binary: Option<PathBuf>,
    doctor: Option<DoctorReport>,
    doctor_error: Option<String>,

    /// The Replace field's buffer. **Never drawn.** [`masked_field`] draws a
    /// function of its length instead.
    pending_key: Option<Secret>,
    key_test: Option<KeyTest>,
    /// True once a `Test` succeeded for [`Self::pending_key`], which is what
    /// unlocks the plain `Replace` commit (§3: "committed only after `Test`
    /// succeeds or the user explicitly saves untested").
    key_tested_ok: bool,
    /// Whether the secret handed to the running `Test` came out of the Replace
    /// field, and so has to be handed back when the check answers. A stored key
    /// is read fresh and dropped instead.
    test_returns_to_pending: bool,
    /// Set by a first `Save` over a settings file that failed to load, so the
    /// second press is the explicit overwrite rather than a silent reset.
    overwrite_armed: bool,

    focus: usize,
    /// Set by Enter; consumed by the control that owns [`Self::focus`].
    activate: bool,
    /// Escape with unsaved edits asks once rather than discarding (§ the design
    /// document's "no silent discard").
    confirm_close: bool,

    scroll: f32,
    /// The content height the previous frame measured. One frame late on
    /// purpose: an immediate-mode body knows its own height only after drawing
    /// it, and a measure pass would double every string measurement.
    content_height: f32,

    background: Option<Background>,
    status: Option<(String, Color)>,
    now: Option<i64>,
    /// `(rail on the left, routing matrix stacked)` from the last drawn frame.
    ///
    /// Reported rather than inferred. Both degradations are decided from the
    /// window size, and a capture cannot tell "the table stacked because the
    /// window is narrow" from "the table is broken" — so the report names which
    /// arrangement is in the picture.
    last_layout: Option<(bool, bool)>,
}

impl Default for AssistSettingsDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AssistSettingsDialog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: false,
            section: Section::Routing,
            widgets: Widgets::new(),
            saved: AssistSettings::default(),
            draft: AssistSettings::default(),
            settings_path: None,
            settings_source: SettingsSource::Absent,
            credentials_path: None,
            credentials: CredentialState::None,
            session_credential: None,
            models_dir: None,
            models_dir_error: None,
            models_dir_field: String::new(),
            models_dir_focused: false,
            catalog: CacheSlot::NeverFetched,
            codex: CacheSlot::NeverFetched,
            codex_binary: None,
            doctor: None,
            doctor_error: None,
            pending_key: None,
            key_test: None,
            key_tested_ok: false,
            test_returns_to_pending: false,
            overwrite_armed: false,
            focus: 0,
            activate: false,
            confirm_close: false,
            scroll: 0.0,
            content_height: 0.0,
            background: None,
            status: None,
            now: None,
            last_layout: None,
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.draft != self.saved
    }

    /// Opens the dialog and reads everything it displays, once.
    ///
    /// `session_fingerprint` is `SessionCredentials::openrouter_fingerprint()` —
    /// the fingerprint only, never the key, so the dialog cannot hold a second
    /// copy of a credential the runtime already owns.
    pub fn open(&mut self, section: Section, session_fingerprint: Option<String>) {
        self.open = true;
        self.section = section;
        self.focus = 0;
        self.scroll = 0.0;
        self.confirm_close = false;
        self.activate = false;
        self.status = None;
        self.key_test = None;
        self.key_tested_ok = false;
        self.pending_key = None;
        self.session_credential = session_fingerprint;
        self.now = now_utc();
        self.reload();
        self.apply_probe_state();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.confirm_close = false;
        // A pending key is dropped rather than retained across an open: a
        // credential that outlives the surface that asked for it is a copy
        // nobody is looking after.
        self.pending_key = None;
        self.key_tested_ok = false;
    }

    /// Re-reads every file the dialog displays.
    fn reload(&mut self) {
        self.settings_path = files::settings_path();
        match &self.settings_path {
            None => {
                self.settings_source = SettingsSource::NoPath;
                self.saved = AssistSettings::default();
            }
            Some(path) => match files::load_settings(path) {
                Ok(Some(settings)) => {
                    self.settings_source = SettingsSource::Loaded;
                    self.saved = settings;
                }
                Ok(None) => {
                    self.settings_source = SettingsSource::Absent;
                    self.saved = AssistSettings::default();
                }
                Err(error) => {
                    // Never a silent reset (§2). The draft still starts from the
                    // defaults so the dialog is usable, but Save is refused until
                    // the user chooses to overwrite.
                    self.settings_source = SettingsSource::Error(error.to_string());
                    self.saved = AssistSettings::default();
                }
            },
        }
        self.draft = self.saved.clone();
        self.models_dir_field = self.draft.local_runtimes.models_dir.clone();

        self.credentials_path = files::credentials_path();
        self.credentials = match &self.credentials_path {
            None => CredentialState::None,
            Some(path) => match files::load_credentials(path) {
                Ok(Some(store)) => match store.get("openrouter", credential_lookup(&self.draft)) {
                    Some(entry) => CredentialState::File {
                        fingerprint: entry.fingerprint(),
                        label: entry.label.clone(),
                    },
                    None => CredentialState::None,
                },
                Ok(None) => CredentialState::None,
                Err(AssistFileError::Permissions(path, mode)) => CredentialState::Refused {
                    path: path.display().to_string(),
                    mode,
                },
                Err(error) => CredentialState::Error(error.to_string()),
            },
        };
        if matches!(self.credentials, CredentialState::None) {
            if let Some(fingerprint) = self.session_credential.clone() {
                self.credentials = CredentialState::Session { fingerprint };
            }
        }

        match models::resolve(Some(&self.draft)) {
            Ok(resolution) => {
                self.models_dir = Some(resolution);
                self.models_dir_error = None;
            }
            Err(error) => {
                self.models_dir = None;
                self.models_dir_error = Some(error.to_string());
            }
        }

        self.catalog = read_cache("openrouter-models-v1.json", |document: &CatalogCache| {
            document.schema_version == "musializer.openrouter-catalog/v1"
        });
        self.codex = read_cache("codex-models-v1.json", |document: &CodexCache| {
            document.schema_version == "musializer.codex-model-catalog/v1"
        });
        self.codex_binary = which("codex");
        self.doctor = None;
        self.doctor_error = None;
        if let Ok(path) = std::env::var(PROBE_DOCTOR_VARIABLE) {
            match std::fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<DoctorReport>(&bytes).map_err(|e| e.to_string())
                }) {
                Ok(report) => self.doctor = Some(report),
                Err(reason) => self.doctor_error = Some(reason),
            }
        }
    }

    /// The probe seams, applied after a real load so a capture photographs the
    /// real surface with one fact changed rather than a mock of it.
    fn apply_probe_state(&mut self) {
        if let Ok(text) = std::env::var(PROBE_KEY_VARIABLE) {
            if !text.is_empty() {
                self.pending_key = Some(Secret::new(text));
            }
        }
        if let Ok(text) = std::env::var(PROBE_DIRTY_VARIABLE) {
            if text.trim() == "1" {
                // One real override through the real path, so the confirm step a
                // capture photographs is the one an edit produces.
                if let Some(mut route) = recommended_route(ContractId::Align) {
                    route.fallback = FallbackPolicy::LocalOnly;
                    set_override(&mut self.draft, route);
                }
                // And a guarantee rather than a hope: a settings file that
                // already carried that exact override would leave the draft
                // clean, and the capture would then photograph the wrong branch
                // while claiming the right one. A gate check that silently tests
                // nothing is the failure this line exists to prevent.
                if !self.is_dirty() {
                    self.draft.catalog.show_experimental = !self.draft.catalog.show_experimental;
                }
            }
        }
        if let Ok(text) = std::env::var(PROBE_TAB_VARIABLE) {
            if let Ok(steps) = text.trim().parse::<u32>() {
                for _ in 0..steps.min(256) {
                    self.focus_next(1);
                }
            }
        }
        if let Ok(text) = std::env::var(PROBE_SCROLL_VARIABLE) {
            if let Ok(offset) = text.trim().parse::<f32>() {
                self.scroll = offset.max(0.0);
            }
        }
        if let Ok(text) = std::env::var(PROBE_ACTIVATE_VARIABLE) {
            if text.trim() == "1" {
                self.activate = true;
            }
        }
        if let Ok(text) = std::env::var(PROBE_ESCAPE_VARIABLE) {
            if text.trim() == "1" {
                self.escape();
            }
        }
    }

    /// Which controls Tab walks, in visual order, for the open section.
    ///
    /// Built from the live model rather than from a constant list: a contract
    /// with no eligible models has no model picker, and a ring that stopped on a
    /// control nobody drew would be a focus ring pointing at nothing.
    #[must_use]
    pub fn focus_order(&self) -> Vec<ControlId> {
        let mut order = vec![FOCUS_SAVE, FOCUS_CLOSE];
        for (index, _) in Section::ALL.iter().enumerate() {
            order.push(FOCUS_TAB_BASE + index as ControlId);
        }
        match self.section {
            Section::Routing => {
                order.push(FOCUS_ROUTING_PROFILE);
                order.push(FOCUS_ROUTING_EXPERIMENTAL);
                for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
                    if contract.is_locked() {
                        continue;
                    }
                    let index = index as ControlId;
                    if contract.eligible_route_types().len() > 1 {
                        order.push(FOCUS_ROUTE_BASE + index);
                    }
                    if !self.model_options_for(contract).is_empty() {
                        order.push(FOCUS_MODEL_BASE + index);
                    }
                    if contract.allowed_fallbacks().len() > 1 {
                        order.push(FOCUS_FALLBACK_BASE + index);
                    }
                }
            }
            Section::LocalModels => {
                order.push(FOCUS_MODELS_DIR);
                order.push(FOCUS_MODELS_GPU);
                order.push(FOCUS_MODELS_STEMS);
                order.push(FOCUS_MODELS_DOCTOR);
            }
            Section::Codex => {
                for (index, _) in CODEX_ELIGIBLE.iter().enumerate() {
                    order.push(FOCUS_CODEX_EFFORT_BASE + index as ControlId);
                }
                order.push(FOCUS_CODEX_REFRESH);
            }
            Section::OpenRouter => {
                order.push(FOCUS_KEY_FIELD);
                order.push(FOCUS_KEY_TEST);
                order.push(FOCUS_KEY_REPLACE);
                order.push(FOCUS_KEY_SAVE_UNTESTED);
                order.push(FOCUS_KEY_FORGET);
                order.push(FOCUS_CATALOG_NETWORK);
                order.push(FOCUS_CATALOG_REFRESH_ON_OPEN);
                order.push(FOCUS_CATALOG_REFRESH);
            }
            Section::Privacy => {
                for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
                    if contract.max_boundary().rank() >= 2 {
                        order.push(FOCUS_PRIVACY_ZDR_BASE + index as ControlId);
                    }
                }
                order.push(FOCUS_PRIVACY_COPY);
            }
        }
        order
    }

    /// The focused control, or `None` when the ring is empty.
    #[must_use]
    pub fn focused(&self) -> Option<ControlId> {
        let order = self.focus_order();
        order.get(self.focus % order.len().max(1)).copied()
    }

    /// Where the ring is, for the report line: `index/total`.
    #[must_use]
    pub fn focus_position(&self) -> (usize, usize) {
        let total = self.focus_order().len();
        (if total == 0 { 0 } else { self.focus % total }, total)
    }

    fn focus_next(&mut self, step: isize) {
        // Traversing is going back to editing, so it cancels a pending "Escape
        // again to discard". Here rather than in the keyboard handler, so a
        // press that moves the focus cancels it too.
        self.confirm_close = false;
        let total = self.focus_order().len();
        if total == 0 {
            self.focus = 0;
            return;
        }
        let current = (self.focus % total) as isize;
        self.focus = (current + step).rem_euclid(total as isize) as usize;
    }

    /// Whether `id` has the focus ring this frame.
    fn is_focused(&self, id: ControlId) -> bool {
        self.focused() == Some(id)
    }

    /// Whether `id` was activated by Enter this frame. Consuming, so one Enter
    /// activates one control.
    fn take_activation(&mut self, id: ControlId) -> bool {
        if self.activate && self.is_focused(id) {
            self.activate = false;
            return true;
        }
        false
    }

    /// Escape: close, or ask first when there are unsaved edits.
    fn escape(&mut self) {
        if self.is_dirty() && !self.confirm_close {
            self.confirm_close = true;
            self.status = Some((
                "Unsaved changes. Escape again to discard, or Save.".to_string(),
                color::ui_warning(),
            ));
            return;
        }
        self.close();
    }

    fn model_options_for(&self, contract: ContractId) -> Vec<String> {
        let resolved = resolve_route(&self.draft, contract);
        let Some(route) = &resolved.route else {
            return Vec::new();
        };
        model_options(
            contract,
            route.route_type,
            self.catalog.document(),
            self.codex.document(),
            self.draft.catalog.show_experimental,
            route.model_id.as_deref(),
        )
    }

    /// Readiness for one resolved route.
    ///
    /// Every "not ready" answer names what to do about it, and every "we have not
    /// looked" answer says so rather than claiming a failure.
    #[must_use]
    pub fn readiness(&self, resolved: &ResolvedRoute) -> Readiness {
        let Some(route) = &resolved.route else {
            return Readiness::Blocked("No route chosen".to_string());
        };
        match route.route_type {
            RouteType::Builtin => Readiness::Ready,
            RouteType::LocalProc => {
                let key = match route.runtime_id.as_str() {
                    "whisper.cpp" => "whisper",
                    "mms-ctc" | "qwen3-fa" => "mms_ctc_aligner",
                    _ => return Readiness::Unknown("Not probed".to_string()),
                };
                match self
                    .doctor
                    .as_ref()
                    .and_then(|report| report.runtimes.get(key))
                {
                    None => Readiness::Unknown("Run doctor".to_string()),
                    Some(identity) if identity.state == "available" => Readiness::Ready,
                    Some(identity) => Readiness::Blocked(
                        identity
                            .remediation
                            .clone()
                            .unwrap_or_else(|| identity.state.clone()),
                    ),
                }
            }
            RouteType::Codex => match &self.codex_binary {
                Some(_) => Readiness::Ready,
                None => Readiness::Blocked("codex not on PATH".to_string()),
            },
            RouteType::OpenRouter => {
                if self.credentials.is_usable() {
                    Readiness::Ready
                } else {
                    Readiness::Blocked("No key".to_string())
                }
            }
        }
    }

    /// The report lines a capture carries as its own evidence.
    ///
    /// One line per concern, so a gate can grep the one it is asserting about
    /// rather than parsing a paragraph. **No line can contain a credential**: the
    /// key appears only as a fingerprint and as the *length* of the mask.
    #[must_use]
    pub fn describe(&self) -> Vec<String> {
        if !self.open {
            return vec!["assist settings: closed".to_string()];
        }
        let (focus, total) = self.focus_position();
        let focused = self
            .focused()
            .map_or_else(|| "none".to_string(), |id| id.to_string());
        let (rail, stacked) = self.last_layout.unwrap_or((true, false));
        let mut lines = vec![format!(
            "assist settings: section={} focus={focus}/{total} control={focused} dirty={} confirm-close={} scroll={:.0} sections={} routing={} busy={}",
            self.section.token(),
            self.is_dirty(),
            self.confirm_close,
            self.scroll,
            if rail { "rail" } else { "tabs" },
            if stacked { "stacked" } else { "matrix" },
            self.background
                .as_ref()
                .map_or("none", Background::label),
        )];
        lines.push(format!(
            "assist settings stores: settings={} path={} credentials={} models-dir={}",
            self.settings_source.token(),
            self.settings_path
                .as_ref()
                .map_or_else(|| "<none>".to_string(), |path| path.display().to_string()),
            self.credentials.token(),
            self.models_dir.as_ref().map_or_else(
                || format!("error({})", self.models_dir_error.as_deref().unwrap_or("?")),
                |resolution| format!(
                    "{} {} writable={}",
                    resolution.source.token(),
                    resolution.path.display(),
                    resolution.writable
                )
            ),
        ));
        let catalog_stamp = self
            .catalog
            .document()
            .and_then(|cache| cache.fetched_at_utc.clone());
        let (catalog_badge, catalog_age) = self.catalog.badge(catalog_stamp.as_deref(), self.now);
        let codex_stamp = self
            .codex
            .document()
            .and_then(|cache| cache.fetched_at_utc.clone());
        let (codex_badge, codex_age) = self.codex.badge(codex_stamp.as_deref(), self.now);
        lines.push(format!(
            "assist settings discovery: catalog={} age=\"{catalog_age}\" models={} codex={} age=\"{codex_age}\" codex-models={} codex-bin={} doctor={}",
            catalog_badge.token(),
            self.catalog
                .document()
                .map_or(0, |cache| cache.models.len()),
            codex_badge.token(),
            self.codex.document().map_or(0, |cache| cache.models.len()),
            self.codex_binary
                .as_ref()
                .map_or_else(|| "absent".to_string(), |path| path.display().to_string()),
            match (&self.doctor, &self.doctor_error) {
                (Some(_), _) => "loaded".to_string(),
                (None, Some(error)) => format!("error({error})"),
                (None, None) => "not run".to_string(),
            },
        ));
        let routes: Vec<String> = ALL_CONTRACTS
            .into_iter()
            .map(|contract| {
                let resolved = resolve_route(&self.draft, contract);
                format!(
                    "{}={}/{}[{}]{}",
                    contract.token(),
                    resolved.compact(),
                    resolved.model_label(),
                    self.readiness(&resolved).token(),
                    if resolved.origin == RouteOrigin::Override {
                        "*"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        lines.push(format!("assist settings routing: {}", routes.join(" ")));
        let masked = masked_field(
            self.pending_key
                .as_ref()
                .map_or(0, |key| key.expose().len()),
        );
        lines.push(format!(
            "assist settings key: state={} masked=\"{masked}\" masked-len={} tested={} test={} network-allowed={} experimental={}",
            self.credentials.token(),
            masked.chars().count(),
            self.key_tested_ok,
            self.key_test
                .as_ref()
                .map_or("none", KeyTest::token),
            self.draft.catalog.network_allowed,
            self.draft.catalog.show_experimental,
        ));
        lines
    }
}

/// The account label a credential is stored under. §2's `lookup_id`, defaulting
/// to `default` — an opaque label, never the secret.
fn credential_lookup(settings: &AssistSettings) -> &str {
    let stored = settings.credentials.openrouter.lookup_id.as_str();
    if stored.is_empty() {
        "default"
    } else {
        stored
    }
}

/// `$XDG_CACHE_HOME/musializer`, else `$HOME/.cache/musializer` — the directory
/// `tools/atomic_cache.py` resolves, spelled the same way.
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

/// The size cap on a cache document, so a corrupt or hostile file cannot make
/// the process allocate. Generous: the live OpenRouter catalog is ~530 KiB and
/// `provider_catalog.py` caps its own fetch at 8 MiB.
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

/// Reads one discovery cache.
///
/// **Absent is [`CacheSlot::NeverFetched`], not an empty document.** That
/// distinction is the honesty rule for this whole section: an empty catalog
/// would read as "OpenRouter offers no models", which is a claim the application
/// has no evidence for.
fn read_cache<T: serde::de::DeserializeOwned>(
    file_name: &str,
    accepts: impl Fn(&T) -> bool,
) -> CacheSlot<T> {
    let Some(path) = cache_dir().map(|dir| dir.join(file_name)) else {
        return CacheSlot::NeverFetched;
    };
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CacheSlot::NeverFetched
        }
        Err(error) => return CacheSlot::Unreadable(error.to_string()),
    };
    if metadata.len() > MAX_CACHE_BYTES {
        return CacheSlot::Unreadable(format!(
            "{} bytes, over the {MAX_CACHE_BYTES} byte cap",
            metadata.len()
        ));
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => return CacheSlot::Unreadable(error.to_string()),
    };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(document) if accepts(&document) => CacheSlot::Loaded(document),
        Ok(_) => CacheSlot::Unreadable("the cache declares another schema".to_string()),
        Err(error) => CacheSlot::Unreadable(error.to_string()),
    }
}

/// Removes every credential-named variable from a child's environment (E1).
///
/// `std::env::vars_os` rather than `vars`: the UTF-8 half **panics** on a
/// variable this process did not choose, and a desktop session is exactly where
/// one turns up. Neither discovery tool needs a credential — the OpenRouter
/// catalog endpoint is unauthenticated and Codex authenticates itself — so this
/// is the same rule `external_analysis.py::_safe_local_env` applies to its own
/// children, spelled for the ones spawned from here.
fn strip_credential_variables(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if musializer_runtime::assist::env::is_credential_named(&name) {
            command.env_remove(name.as_ref());
        }
    }
}

/// The first `PATH` entry holding an executable called `name`.
///
/// Membership only — nothing is run, and the answer is used to say "codex is not
/// on PATH" rather than to invent a version.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// `tools/<name>`, resolved the way `find_assist_helper` resolves the analysis
/// helper (`process::font_import::find_helper`): beside the executable, one and
/// two directories up, then the working directory. Duplicated rather than
/// shared because that function is private and takes an application directory
/// this surface is not handed.
fn find_tool(name: &str) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok();
    let directory = executable.as_deref().and_then(Path::parent);
    let mut candidates = Vec::new();
    if let Some(directory) = directory {
        candidates.push(directory.join("tools").join(name));
        candidates.push(directory.join("..").join("tools").join(name));
        candidates.push(directory.join("../..").join("tools").join(name));
    }
    candidates.push(PathBuf::from("./tools").join(name));
    candidates.into_iter().find(|path| path.is_file())
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The widget-id namespace for every control in the dialog.
///
/// The dialog owns its own [`Widgets`] bank, so this cannot collide with a
/// panel's; it is still a distinct namespace because two controls *inside* the
/// dialog colliding would let one release another's press, which is the same
/// defect the shell's namespaces exist to prevent.
const DIALOG_WIDGETS: u32 = 128;

/// The shell-side blocker's namespace. Drawn into the **shell's** bank, which is
/// how every other widget in the frame is prevented from claiming a press.
pub const MODAL_BLOCK_NAMESPACE: u32 = 129;

/// Row metrics. One place, so the focus ring, the hit box and the drawn box
/// cannot drift.
const ROW_HEIGHT: f32 = 26.0;
const ROW_GAP: f32 = 6.0;
/// One matrix row, including the room the suitability marker takes under the
/// MODEL picker. Wider than `ROW_HEIGHT + ROW_GAP` because that marker is drawn
/// *below* its control, and at the plain gap it overlapped the row beneath — a
/// capture is what caught it.
const MATRIX_ROW_PITCH: f32 = 38.0;
/// The suitability marker's size. The smallest size the native interface bank
/// carries; anything below it is a scaled atlas the headless gate refuses.
const MARKER_FONT_SIZE: f32 = 11.0;
const CONTROL_HEIGHT: f32 = 26.0;

impl AssistSettingsDialog {
    /// One frame of the modal: keyboard, background polling, and pixels.
    ///
    /// Called last in [`crate::ui::shell::Shell::draw`], after the notice tray
    /// and before the shell's tooltip, so the dialog is above everything.
    pub fn draw(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        window: (f32, f32),
        ui_scale: UiScale,
    ) {
        if !self.open {
            return;
        }
        self.poll_background(d);
        self.keyboard(d);
        if !self.open {
            return;
        }
        self.widgets.begin_frame(ui_scale);
        let layout = DialogLayout::of(window);
        self.last_layout = Some((layout.rail_on_left, layout.routing_compact()));
        let font = fonts.ui();

        // The scrim is opaque enough that the workspace behind it reads as
        // context rather than as content. It is not a widget: the shell's own
        // blocker already claimed this frame's press.
        widgets::fill(d, layout.scrim, Color::new(10, 11, 14, 190));
        widgets::fill(d, layout.dialog, color::ui_surface());
        d.draw_rectangle_lines_ex(widgets::rectangle(layout.dialog), 1.0, color::ui_rule());

        self.draw_header(d, fonts, layout);
        self.draw_tabs(d, font, layout);
        self.draw_body(d, fonts, layout, ui_scale);
        self.draw_footer(d, font, layout);

        if let Some(tooltip) = self.widgets.tooltip().cloned() {
            widgets::draw_tooltip(d, font, &tooltip, window);
        }
        // An Enter nobody claimed is spent rather than carried: a pending
        // activation surviving a section change would fire on whatever control
        // inherited the focus index.
        self.activate = false;
        report(&self.describe());
    }

    fn draw_header(&mut self, d: &mut RaylibDrawHandle<'_>, fonts: &Faces, layout: DialogLayout) {
        let font = fonts.ui();
        widgets::fill(d, layout.header, color::ui_raised());
        d.draw_line_ex(
            Vector2::new(layout.header.x, layout.header.y + layout.header.height),
            Vector2::new(
                layout.header.x + layout.header.width,
                layout.header.y + layout.header.height,
            ),
            1.0,
            color::ui_rule(),
        );
        widgets::draw_text(
            d,
            font,
            "AI SETTINGS",
            layout.dialog.x + DIALOG_PADDING,
            layout.dialog.y + 14.0,
            metric::UI_FONT_HEADER,
            color::accent(),
        );
        let subtitle = if self.is_dirty() {
            "Unsaved changes \u{00b7} they apply to the next job"
        } else {
            "Settings only \u{00b7} opening this never starts an analysis"
        };
        draw_subtitle(d, font, &layout, subtitle, self.is_dirty());

        let save_enabled =
            self.is_dirty() && !matches!(self.settings_source, SettingsSource::NoPath);
        if save_enabled {
            let clicked = self
                .widgets
                .text_button(
                    d,
                    font,
                    widgets::widget_id(DIALOG_WIDGETS, 1),
                    layout.save,
                    "Save",
                    true,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_VALUE),
                )
                .clicked;
            self.focus_ring(d, layout.save, FOCUS_SAVE);
            if clicked || self.take_activation(FOCUS_SAVE) {
                self.save();
            }
        } else {
            self.widgets
                .disabled_button(d, font, layout.save, "Save", Some(metric::UI_FONT_VALUE));
            self.focus_ring(d, layout.save, FOCUS_SAVE);
            let _ = self.take_activation(FOCUS_SAVE);
        }

        let close = self.widgets.text_button(
            d,
            font,
            widgets::widget_id(DIALOG_WIDGETS, 2),
            layout.close,
            "\u{00d7}",
            false,
            ButtonStyle::Neutral,
            Some(18.0),
        );
        self.widgets.hint(
            d,
            close,
            widgets::widget_id(DIALOG_WIDGETS, 2),
            layout.close,
            "Close AI settings [Esc]",
        );
        self.focus_ring(d, layout.close, FOCUS_CLOSE);
        if close.clicked || self.take_activation(FOCUS_CLOSE) {
            self.escape();
        }
    }

    fn draw_tabs(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        layout: DialogLayout,
    ) {
        if layout.rail_on_left {
            // The rail's own column, so the body's left edge is a rule rather
            // than a guessed gap.
            d.draw_line_ex(
                Vector2::new(layout.content.x, layout.content.y),
                Vector2::new(layout.content.x, layout.content.y + layout.content.height),
                1.0,
                color::ui_rule(),
            );
        }
        for (index, section) in Section::ALL.into_iter().enumerate() {
            let boundary = layout.tabs[index];
            if boundary.is_empty() {
                continue;
            }
            let id = widgets::widget_id(DIALOG_WIDGETS, 10 + index as u32);
            let clicked = self
                .widgets
                .text_button(
                    d,
                    font,
                    id,
                    boundary,
                    section.title(),
                    self.section == section,
                    ButtonStyle::Neutral,
                    Some(metric::UI_FONT_CAPTION),
                )
                .clicked;
            self.focus_ring(d, boundary, FOCUS_TAB_BASE + index as ControlId);
            if (clicked || self.take_activation(FOCUS_TAB_BASE + index as ControlId))
                && self.section != section
            {
                self.section = section;
                self.scroll = 0.0;
                self.focus = 0;
            }
        }
    }

    fn draw_footer(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        layout: DialogLayout,
    ) {
        widgets::fill(d, layout.footer, color::ui_raised());
        d.draw_line_ex(
            Vector2::new(layout.footer.x, layout.footer.y),
            Vector2::new(layout.footer.x + layout.footer.width, layout.footer.y),
            1.0,
            color::ui_rule(),
        );
        let (text, tint) = match (&self.status, &self.background) {
            (_, Some(job)) => (format!("Working: {}\u{2026}", job.label()), color::accent()),
            (Some((text, tint)), None) => (text.clone(), *tint),
            (None, None) => (
                "Tab moves between controls \u{00b7} Enter activates \u{00b7} Esc closes"
                    .to_string(),
                color::ui_muted(),
            ),
        };
        widgets::draw_text(
            d,
            font,
            &text,
            layout.footer.x + DIALOG_PADDING,
            layout.footer.y + 10.0,
            metric::UI_FONT_CAPTION,
            tint,
        );
    }

    fn draw_body(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        layout: DialogLayout,
        ui_scale: UiScale,
    ) {
        let body = layout.body();
        if body.is_empty() {
            return;
        }
        // The scroll bound uses the height the *previous* frame measured; see
        // the field comment. It is clamped before drawing, so a section that got
        // shorter cannot leave the view past its own end for a frame.
        let overflow = (self.content_height - body.height).max(0.0);
        self.scroll = self.scroll.clamp(0.0, overflow);

        let pointer = ui_scale.mouse(d);
        if layout.content.contains_point(pointer.x, pointer.y) {
            let wheel = d.get_mouse_wheel_move();
            if wheel.abs() > f32::EPSILON {
                self.scroll = (self.scroll - wheel * 48.0).clamp(0.0, overflow);
            }
        }

        let mut clip = widgets::begin_scissor(d, layout.content, ui_scale);
        let top = body.y - self.scroll;
        let mut cursor = top;
        let inner = UiRect::new(body.x, body.y, body.width, body.height);
        match self.section {
            Section::Routing => self.draw_routing(&mut clip, fonts, inner, &mut cursor, &layout),
            Section::LocalModels => self.draw_local_models(&mut clip, fonts, inner, &mut cursor),
            Section::Codex => self.draw_codex(&mut clip, fonts, inner, &mut cursor),
            Section::OpenRouter => self.draw_openrouter(&mut clip, fonts, inner, &mut cursor),
            Section::Privacy => self.draw_privacy(&mut clip, fonts, inner, &mut cursor),
        }
        self.content_height = cursor - top + 16.0;
        drop(clip);

        // The scrollbar is drawn outside the scissor so it is never clipped by
        // the content it describes.
        if overflow > 0.0 {
            let track = UiRect::new(
                layout.content.x + layout.content.width - 8.0,
                layout.content.y + 4.0,
                4.0,
                (layout.content.height - 8.0).max(0.0),
            );
            widgets::fill(d, track, color::ui_rule());
            let visible = (body.height / self.content_height).clamp(0.05, 1.0);
            let thumb_height = (track.height * visible).max(24.0);
            let travel = (track.height - thumb_height).max(0.0);
            let progress = if overflow > 0.0 {
                self.scroll / overflow
            } else {
                0.0
            };
            widgets::fill(
                d,
                UiRect::new(
                    track.x,
                    track.y + travel * progress,
                    track.width,
                    thumb_height,
                ),
                color::ui_disabled(),
            );
        }
    }

    // -- shared drawing pieces ---------------------------------------------

    fn focus_ring(&self, d: &mut RaylibDrawHandle<'_>, boundary: UiRect, id: ControlId) {
        if !self.is_focused(id) || boundary.is_empty() {
            return;
        }
        d.draw_rectangle_lines_ex(
            widgets::rectangle(UiRect::new(
                boundary.x - 3.0,
                boundary.y - 3.0,
                boundary.width + 6.0,
                boundary.height + 6.0,
            )),
            2.0,
            color::accent(),
        );
    }

    /// A labelled cycling picker: one press advances to the next option.
    ///
    /// A cycle rather than a dropdown because this shell has no popup layer and
    /// inventing one for five pickers would be a second widget vocabulary. Every
    /// option is reachable, the current one is always drawn, and the tooltip
    /// names how many there are so the control is not a mystery.
    #[allow(
        clippy::too_many_arguments,
        reason = "the widget shape used throughout this shell: target, face, id, box, label, state"
    )]
    fn picker(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        widget: u32,
        focus: ControlId,
        boundary: UiRect,
        label: &str,
        tip: &str,
        enabled: bool,
    ) -> bool {
        if boundary.is_empty() {
            return false;
        }
        if !enabled {
            self.widgets
                .disabled_button(d, font, boundary, label, Some(metric::UI_FONT_CAPTION));
            self.focus_ring(d, boundary, focus);
            let _ = self.take_activation(focus);
            return false;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, widget);
        let state = self.widgets.text_button(
            d,
            font,
            id,
            boundary,
            label,
            false,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(d, state, id, boundary, tip);
        self.focus_ring(d, boundary, focus);
        state.clicked || self.take_activation(focus)
    }

    /// A two-state control drawn as its own value, which is how every toggle in
    /// this shell reads: the label says what it *is*, not what pressing it does.
    #[allow(clippy::too_many_arguments, reason = "same shape as `picker`")]
    fn toggle(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        widget: u32,
        focus: ControlId,
        boundary: UiRect,
        label: &str,
        on: bool,
        tip: &str,
    ) -> bool {
        if boundary.is_empty() {
            return false;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, widget);
        let state = self.widgets.text_button(
            d,
            font,
            id,
            boundary,
            label,
            on,
            ButtonStyle::Neutral,
            Some(metric::UI_FONT_CAPTION),
        );
        self.widgets.hint(d, state, id, boundary, tip);
        self.focus_ring(d, boundary, focus);
        state.clicked || self.take_activation(focus)
    }
}

/// The header's second line. A free function because it needs neither the
/// dialog's state nor its widget bank, and threading `&mut self` through it
/// would borrow the bank for a string.
fn draw_subtitle(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    layout: &DialogLayout,
    text: &str,
    dirty: bool,
) {
    widgets::draw_text(
        d,
        font,
        text,
        layout.dialog.x + DIALOG_PADDING + 132.0,
        layout.dialog.y + 18.0,
        metric::UI_FONT_CAPTION,
        if dirty {
            color::ui_warning()
        } else {
            color::ui_muted()
        },
    );
}

/// Prints each report line once, when it changes.
///
/// The same rule and the same reason as `panels/assist.rs`'s `report_review`: a
/// line printed every frame is a log, not a report, and a capture's own evidence
/// has to be greppable.
fn report(lines: &[String]) {
    thread_local! {
        static LAST: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        if *last != lines {
            for line in lines {
                println!("{line}");
            }
            *last = lines.to_vec();
        }
    });
}

// ---------------------------------------------------------------------------
// Section bodies
// ---------------------------------------------------------------------------

/// A section heading plus its explanatory line, and the cursor advance.
fn heading(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    title: &str,
    subtitle: &str,
) {
    widgets::draw_text(
        d,
        font,
        title,
        body.x,
        *cursor,
        metric::UI_FONT_HEADER,
        color::accent(),
    );
    *cursor += 26.0;
    if !subtitle.is_empty() {
        for line in wrap(font, subtitle, body.width, metric::UI_FONT_CAPTION) {
            widgets::draw_text(
                d,
                font,
                &line,
                body.x,
                *cursor,
                metric::UI_FONT_CAPTION,
                color::ui_muted(),
            );
            *cursor += 17.0;
        }
    }
    *cursor += 8.0;
}

/// A minor heading inside a section.
fn subheading(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    title: &str,
) {
    *cursor += 6.0;
    widgets::draw_text(
        d,
        font,
        title,
        body.x,
        *cursor,
        metric::UI_FONT_LABEL,
        color::accent(),
    );
    *cursor += 22.0;
    d.draw_line_ex(
        Vector2::new(body.x, *cursor - 4.0),
        Vector2::new(body.x + body.width, *cursor - 4.0),
        1.0,
        color::ui_rule(),
    );
    *cursor += 4.0;
}

/// A `label: value` line, with the value in ink and the label muted.
fn field_line(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    label: &str,
    value: &str,
    tint: Color,
) {
    widgets::draw_text(
        d,
        font,
        label,
        body.x,
        *cursor,
        metric::UI_FONT_CAPTION,
        color::ui_muted(),
    );
    let offset = 186.0f32.min(body.width * 0.4);
    let available = (body.width - offset).max(24.0);
    widgets::draw_text(
        d,
        font,
        &ellipsize(font, value, available, metric::UI_FONT_CAPTION),
        body.x + offset,
        *cursor,
        metric::UI_FONT_CAPTION,
        tint,
    );
    *cursor += 19.0;
}

/// A muted paragraph, wrapped to the body width.
fn paragraph(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    text: &str,
    tint: Color,
) {
    for line in wrap(font, text, body.width, metric::UI_FONT_CAPTION) {
        widgets::draw_text(
            d,
            font,
            &line,
            body.x,
            *cursor,
            metric::UI_FONT_CAPTION,
            tint,
        );
        *cursor += 17.0;
    }
}

/// A small filled badge: the stale/offline marker, the LOCKED marker, a
/// readiness pill.
fn badge(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    boundary: UiRect,
    text: &str,
    tint: Color,
) {
    if boundary.is_empty() {
        return;
    }
    d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, tint);
    let size = 11.0;
    let fitted = ellipsize(font, text, (boundary.width - 8.0).max(8.0), size);
    widgets::draw_text(
        d,
        font,
        &fitted,
        boundary.x + 4.0,
        boundary.y + (boundary.height - size) * 0.5,
        size,
        tint,
    );
}

/// Greedy word wrap against the measured face.
fn wrap(
    font: &musializer_runtime::font::UiFonts,
    text: &str,
    width: f32,
    size: f32,
) -> Vec<String> {
    if width <= 0.0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if widgets::measure(font, &candidate, size) <= width || current.is_empty() {
            current = candidate;
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Truncates with an ellipsis rather than letting a value print past its column.
fn ellipsize(
    font: &musializer_runtime::font::UiFonts,
    text: &str,
    width: f32,
    size: f32,
) -> String {
    if width <= 0.0 {
        return String::new();
    }
    if widgets::measure(font, text, size) <= width {
        return text.to_string();
    }
    let mut fitted = String::new();
    for character in text.chars() {
        let candidate = format!("{fitted}{character}\u{2026}");
        if widgets::measure(font, &candidate, size) > width {
            break;
        }
        fitted.push(character);
    }
    fitted.push('\u{2026}');
    fitted
}

impl AssistSettingsDialog {
    // -- 1. Routing --------------------------------------------------------

    fn draw_routing(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        body: UiRect,
        cursor: &mut f32,
        layout: &DialogLayout,
    ) {
        let font = fonts.ui();
        heading(
            d, font, body, cursor,
            Section::Routing.heading(),
            "One row per task contract. Every change applies to the next job \u{2014} a job already \
             running keeps the routes it started with.",
        );

        // Profile and the experimental gate.
        let profile_label = format!(
            "Profile: {}",
            if self.draft.active_profile == "recommended" {
                "Recommended (built in)".to_string()
            } else {
                format!("{} (per-task overrides)", self.draft.active_profile)
            }
        );
        let profile_box = UiRect::new(body.x, *cursor, 250.0f32.min(body.width), CONTROL_HEIGHT);
        let has_overrides = self.draft.profile(OVERRIDE_PROFILE_ID).is_some();
        if self.picker(
            d,
            font,
            20,
            FOCUS_ROUTING_PROFILE,
            profile_box,
            &profile_label,
            "Switch between the built-in Recommended profile and your per-task overrides",
            has_overrides,
        ) {
            self.draft.active_profile = if self.draft.active_profile == "recommended" {
                OVERRIDE_PROFILE_ID.to_string()
            } else {
                "recommended".to_string()
            };
        }
        let experimental_box = UiRect::new(
            body.x + profile_box.width + 8.0,
            *cursor,
            230.0f32.min((body.width - profile_box.width - 8.0).max(0.0)),
            CONTROL_HEIGHT,
        );
        let show_experimental = self.draft.catalog.show_experimental;
        if self.toggle(
            d,
            font,
            21,
            FOCUS_ROUTING_EXPERIMENTAL,
            experimental_box,
            if show_experimental {
                "Show experimental: ON"
            } else {
                "Show experimental: off"
            },
            show_experimental,
            "Unmeasured models are hidden by default. Turning this on offers them with a warning; \
             a model measured and failed is never offered.",
        ) {
            self.draft.catalog.show_experimental = !show_experimental;
        }
        *cursor += CONTROL_HEIGHT + 6.0;
        if show_experimental {
            paragraph(
                d,
                font,
                body,
                cursor,
                "Warning: experimental models have not passed a Musializer benchmark for the \
                 contract they are offered under. Results are unmeasured.",
                color::ui_warning(),
            );
        }
        *cursor += 6.0;

        if layout.routing_compact() {
            self.draw_routing_compact(d, font, body, cursor);
        } else {
            self.draw_routing_matrix(d, font, body, cursor);
        }

        *cursor += 8.0;
        paragraph(
            d,
            font,
            body,
            cursor,
            "TC-MEASURED is the built-in deterministic analyzer. It is shown so the whole pipeline \
             is visible, and it is locked: it is not user-routable.",
            color::ui_muted(),
        );
        paragraph(
            d,
            font,
            body,
            cursor,
            "A * marks a per-task override. Fallback never raises the data boundary on its own: \
             only Ask can, and only after you answer it.",
            color::ui_muted(),
        );
    }

    /// Column widths for the six-column matrix. Derived from the body width so
    /// the flexible MODEL column absorbs the slack rather than every column
    /// drifting.
    fn routing_columns(width: f32) -> [f32; 6] {
        let fixed = [104.0f32, 132.0, 118.0, 106.0, 96.0];
        let gaps = 6.0 * 5.0;
        let model = (width - fixed.iter().sum::<f32>() - gaps).max(90.0);
        [fixed[0], fixed[1], fixed[2], model, fixed[3], fixed[4]]
    }

    fn draw_routing_matrix(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let columns = Self::routing_columns(body.width);
        let headers = [
            "CONTRACT", "BOUNDARY", "ROUTE", "MODEL", "FALLBACK", "READY",
        ];
        let mut x = body.x;
        for (index, header) in headers.iter().enumerate() {
            widgets::draw_text(d, font, header, x, *cursor, 11.0, color::ui_muted());
            x += columns[index] + 6.0;
        }
        *cursor += 16.0;
        d.draw_line_ex(
            Vector2::new(body.x, *cursor - 3.0),
            Vector2::new(body.x + body.width, *cursor - 3.0),
            1.0,
            color::ui_rule(),
        );

        for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
            let row_y = *cursor;
            let resolved = resolve_route(&self.draft, contract);
            let readiness = self.readiness(&resolved);
            let mut x = body.x;

            let name = if resolved.origin == RouteOrigin::Override {
                format!("{}*", contract.token())
            } else {
                contract.token().to_string()
            };
            widgets::draw_text(
                d,
                font,
                &name,
                x,
                row_y + 6.0,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            x += columns[0] + 6.0;

            let boundary = resolved.boundary();
            widgets::draw_text(
                d,
                font,
                boundary.token(),
                x,
                row_y + 6.0,
                metric::UI_FONT_CAPTION,
                if boundary.rank() >= 2 {
                    color::ui_warning()
                } else if boundary.rank() == 1 {
                    color::ui_ink()
                } else {
                    color::ui_success()
                },
            );
            x += columns[1] + 6.0;

            self.routing_route_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(x, row_y, columns[2], ROW_HEIGHT),
                &resolved,
            );
            x += columns[2] + 6.0;
            self.routing_model_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(x, row_y, columns[3], ROW_HEIGHT),
                &resolved,
            );
            x += columns[3] + 6.0;
            self.routing_fallback_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(x, row_y, columns[4], ROW_HEIGHT),
                &resolved,
            );
            x += columns[4] + 6.0;
            badge(
                d,
                font,
                UiRect::new(x, row_y + 3.0, columns[5], 20.0),
                &readiness.label(),
                readiness.tint(),
            );

            *cursor += MATRIX_ROW_PITCH;
        }
    }

    fn draw_routing_compact(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        body: UiRect,
        cursor: &mut f32,
    ) {
        // Two lines per contract rather than six columns. The picker set is
        // identical: a narrow window loses the table, never a control.
        for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
            let resolved = resolve_route(&self.draft, contract);
            let readiness = self.readiness(&resolved);
            let name = if resolved.origin == RouteOrigin::Override {
                format!("{}*", contract.token())
            } else {
                contract.token().to_string()
            };
            widgets::draw_text(
                d,
                font,
                &name,
                body.x,
                *cursor,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            widgets::draw_text(
                d,
                font,
                resolved.boundary().token(),
                body.x + 110.0,
                *cursor,
                11.0,
                color::ui_muted(),
            );
            badge(
                d,
                font,
                UiRect::new(
                    body.x + body.width - 96.0,
                    *cursor - 3.0,
                    96.0f32.min(body.width),
                    18.0,
                ),
                &readiness.label(),
                readiness.tint(),
            );
            *cursor += 20.0;
            let third = ((body.width - 12.0) / 3.0).max(60.0);
            self.routing_route_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(body.x, *cursor, third, ROW_HEIGHT),
                &resolved,
            );
            self.routing_model_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(body.x + third + 6.0, *cursor, third, ROW_HEIGHT),
                &resolved,
            );
            self.routing_fallback_cell(
                d,
                font,
                contract,
                index,
                UiRect::new(body.x + (third + 6.0) * 2.0, *cursor, third, ROW_HEIGHT),
                &resolved,
            );
            // The same room the matrix leaves for the suitability marker under
            // the MODEL picker: at the plain gap it printed into the next
            // contract's name, which a capture caught and no test could have.
            *cursor += MATRIX_ROW_PITCH + 8.0;
        }
    }

    fn routing_route_cell(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        contract: ContractId,
        index: usize,
        boundary: UiRect,
        resolved: &ResolvedRoute,
    ) {
        let eligible = contract.eligible_route_types();
        let enabled = !contract.is_locked() && eligible.len() > 1;
        let label = if contract.is_locked() {
            "locked".to_string()
        } else {
            resolved.route.as_ref().map_or_else(
                || "choose".to_string(),
                |route| route.route_type.token().to_string(),
            )
        };
        let tip = format!(
            "{}: {} eligible route type(s) \u{00b7} caps at {}",
            contract.token(),
            eligible.len(),
            contract.max_boundary().token()
        );
        if self.picker(
            d,
            font,
            100 + index as u32,
            FOCUS_ROUTE_BASE + index as ControlId,
            boundary,
            &label,
            &tip,
            enabled,
        ) {
            let current = resolved.route.as_ref().map(|route| route.route_type);
            if let Some(next) = cycle(eligible, current.as_ref()) {
                let mut route = resolved.route.clone().unwrap_or(Route {
                    contract,
                    route_type: next,
                    runtime_id: default_runtime_id(next),
                    model_id: None,
                    model_path: None,
                    reasoning_effort: None,
                    fallback: contract.allowed_fallbacks()[0],
                    provider: None,
                });
                route.route_type = next;
                route.runtime_id = default_runtime_id(next);
                route.model_id = None;
                route.reasoning_effort =
                    (next == RouteType::Codex).then_some(ReasoningEffort::Medium);
                route.provider =
                    (next == RouteType::OpenRouter).then(|| Provider::defaults_for(contract));
                if !contract.allowed_fallbacks().contains(&route.fallback) {
                    route.fallback = contract.allowed_fallbacks()[0];
                }
                set_override(&mut self.draft, route);
            }
        }
    }

    fn routing_model_cell(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        contract: ContractId,
        index: usize,
        boundary: UiRect,
        resolved: &ResolvedRoute,
    ) {
        let options = self.model_options_for(contract);
        let label = resolved.model_label();
        let status = resolved
            .route
            .as_ref()
            .and_then(|route| route.model_id.as_deref())
            .map(|id| suitability::status(id, contract));
        let tip = match status {
            Some(Suitability::Recommended) => format!(
                "{label}: recommended for {} on this repository's own benchmark",
                contract.token()
            ),
            Some(Suitability::Experimental) => format!(
                "{label}: experimental \u{2014} no Musializer benchmark for {}",
                contract.token()
            ),
            Some(Suitability::Unsupported) => format!(
                "{label}: measured and failed for {}; it is never offered",
                contract.token()
            ),
            None => format!("{} models offered for {}", options.len(), contract.token()),
        };
        let enabled = !contract.is_locked() && options.len() > 1;
        if self.picker(
            d,
            font,
            200 + index as u32,
            FOCUS_MODEL_BASE + index as ControlId,
            boundary,
            &label,
            &tip,
            enabled,
        ) {
            let current = resolved
                .route
                .as_ref()
                .and_then(|route| route.model_id.clone());
            if let Some(next) = cycle(&options, current.as_ref()) {
                if let Some(mut route) = resolved.route.clone() {
                    route.model_id =
                        if route.route_type == RouteType::Codex && next == CODEX_DEFAULT_LABEL {
                            // §5 rule 6: the fallback label is the *absence* of a
                            // model id, never a model id spelled "Codex default".
                            None
                        } else {
                            Some(next)
                        };
                    set_override(&mut self.draft, route);
                }
            }
        }
        // The suitability marker sits under the picker rather than inside it, so
        // the model id keeps the whole button width.
        //
        // 11 px, not 10: the interface bank is rasterized at native sizes and
        // the gate refuses any request outside it, because a scaled atlas is the
        // blur this shell rebuilt its typography to delete. The first draft asked
        // for 10 and the run reported `non-native-requests=40`.
        if let Some(status) = status {
            if status != Suitability::Recommended {
                widgets::draw_text(
                    d,
                    font,
                    status.token(),
                    boundary.x + 2.0,
                    boundary.y + boundary.height + 1.0,
                    MARKER_FONT_SIZE,
                    if status == Suitability::Unsupported {
                        color::ui_danger()
                    } else {
                        color::ui_warning()
                    },
                );
            }
        }
    }

    fn routing_fallback_cell(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        contract: ContractId,
        index: usize,
        boundary: UiRect,
        resolved: &ResolvedRoute,
    ) {
        let allowed = contract.allowed_fallbacks();
        let enabled = !contract.is_locked() && allowed.len() > 1;
        let label = resolved.route.as_ref().map_or_else(
            || "\u{2014}".to_string(),
            |route| route.fallback.token().to_string(),
        );
        let tip = format!(
            "{}: {}",
            contract.token(),
            allowed
                .iter()
                .map(|policy| policy.token())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if self.picker(
            d,
            font,
            300 + index as u32,
            FOCUS_FALLBACK_BASE + index as ControlId,
            boundary,
            &label,
            &tip,
            enabled,
        ) {
            let current = resolved.route.as_ref().map(|route| route.fallback);
            if let Some(next) = cycle(allowed, current.as_ref()) {
                if let Some(mut route) = resolved.route.clone() {
                    route.fallback = next;
                    set_override(&mut self.draft, route);
                }
            }
        }
    }
}

/// The runtime id a freshly chosen route type starts with.
fn default_runtime_id(route_type: RouteType) -> String {
    match route_type {
        RouteType::Builtin => "builtin".to_string(),
        RouteType::LocalProc => "whisper.cpp".to_string(),
        RouteType::Codex => "codex".to_string(),
        RouteType::OpenRouter => "openrouter".to_string(),
    }
}

impl AssistSettingsDialog {
    // -- 2. Local models ---------------------------------------------------

    fn draw_local_models(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let font = fonts.ui();
        heading(
            d,
            font,
            body,
            cursor,
            Section::LocalModels.heading(),
            "Where downloaded weights live, and what the installed runtimes actually are. Nothing \
             here is guessed: a runtime that has not been probed says so.",
        );

        subheading(d, font, body, cursor, "Models directory");
        match (&self.models_dir, &self.models_dir_error) {
            (Some(resolution), _) => {
                let resolution = resolution.clone();
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Resolved",
                    &resolution.path.display().to_string(),
                    color::ui_ink(),
                );
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Chosen by",
                    resolution.source.token(),
                    color::ui_ink(),
                );
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Writable",
                    if resolution.writable { "yes" } else { "no" },
                    if resolution.writable {
                        color::ui_success()
                    } else {
                        color::ui_danger()
                    },
                );
                // Every losing candidate, because the operator rule is "never a
                // location the user was not shown".
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Install default",
                    &resolution.install_default.as_ref().map_or_else(
                        || "unknown".to_string(),
                        |path| {
                            format!(
                                "{} ({})",
                                path.display(),
                                if resolution.install_default_writable {
                                    "writable"
                                } else {
                                    "not writable"
                                }
                            )
                        },
                    ),
                    color::ui_muted(),
                );
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Home fallback",
                    &resolution
                        .home_fallback
                        .as_ref()
                        .map_or_else(|| "unknown".to_string(), |path| path.display().to_string()),
                    color::ui_muted(),
                );
            }
            (None, Some(error)) => {
                paragraph(d, font, body, cursor, error, color::ui_danger());
            }
            (None, None) => {
                paragraph(d, font, body, cursor, "Not resolved.", color::ui_muted());
            }
        }
        *cursor += 4.0;
        widgets::draw_text(
            d,
            font,
            "Override",
            body.x,
            *cursor + 6.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        let field = UiRect::new(
            body.x + 90.0,
            *cursor,
            (body.width - 90.0).max(60.0),
            CONTROL_HEIGHT,
        );
        self.draw_path_field(d, font, field);
        *cursor += CONTROL_HEIGHT + 6.0;
        paragraph(
            d,
            font,
            body,
            cursor,
            "An override wins over the default above, writable or not \u{2014} substituting one \
             silently would move your weights somewhere you never chose.",
            color::ui_muted(),
        );

        subheading(d, font, body, cursor, "Runtimes");
        match (&self.doctor, &self.doctor_error) {
            (None, None) => paragraph(
                d,
                font,
                body,
                cursor,
                "Not probed. Run the doctor to read the installed Whisper, aligner and separator. \
                 This spawns tools/musializer_doctor.py; it starts no analysis.",
                color::ui_muted(),
            ),
            (None, Some(error)) => paragraph(
                d,
                font,
                body,
                cursor,
                &format!("The doctor report could not be read: {error}"),
                color::ui_danger(),
            ),
            (Some(report), _) => {
                let report = report.clone();
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Report schema",
                    if report.schema_version.is_empty() {
                        "not reported"
                    } else {
                        report.schema_version.as_str()
                    },
                    color::ui_muted(),
                );
                for (key, label) in RUNTIME_ROWS {
                    subheading_row(d, font, body, cursor, label);
                    match report.runtimes.get(key) {
                        None => paragraph(
                            d,
                            font,
                            body,
                            cursor,
                            "the doctor report has no entry for this runtime",
                            color::ui_muted(),
                        ),
                        Some(identity) => {
                            field_line(
                                d,
                                font,
                                body,
                                cursor,
                                "State",
                                &identity.state,
                                if identity.state == "available" {
                                    color::ui_success()
                                } else {
                                    color::ui_warning()
                                },
                            );
                            for (name, value) in [
                                ("Path", identity.path.clone()),
                                ("Version", identity.version.clone()),
                                ("Model", identity.model_path.clone()),
                                ("Model sha256", identity.model_sha256.clone()),
                                ("Languages", identity.language_support.clone()),
                                (
                                    "GPU",
                                    identity.gpu_ready.map(|ready| {
                                        if ready { "ready" } else { "not ready" }.to_string()
                                    }),
                                ),
                            ] {
                                // An absent field prints "not reported" rather
                                // than an empty value that reads as "none".
                                field_line(
                                    d,
                                    font,
                                    body,
                                    cursor,
                                    name,
                                    value.as_deref().unwrap_or("not reported"),
                                    if value.is_some() {
                                        color::ui_ink()
                                    } else {
                                        color::ui_muted()
                                    },
                                );
                            }
                            if let Some(remediation) = &identity.remediation {
                                paragraph(d, font, body, cursor, remediation, color::ui_warning());
                            }
                        }
                    }
                }
            }
        }

        *cursor += 6.0;
        let gpu = self.draft.local_runtimes.prefer_gpu;
        let gpu_box = UiRect::new(body.x, *cursor, 160.0f32.min(body.width), CONTROL_HEIGHT);
        if self.toggle(
            d,
            font,
            30,
            FOCUS_MODELS_GPU,
            gpu_box,
            if gpu {
                "Prefer GPU: ON"
            } else {
                "Prefer GPU: off"
            },
            gpu,
            "Ask the local runtimes for a GPU device when one is available",
        ) {
            self.draft.local_runtimes.prefer_gpu = !gpu;
        }
        let stems = self.draft.local_runtimes.stem_separation;
        let stems_box = UiRect::new(
            body.x + gpu_box.width + 8.0,
            *cursor,
            210.0f32.min((body.width - gpu_box.width - 8.0).max(0.0)),
            CONTROL_HEIGHT,
        );
        if self.picker(
            d,
            font,
            31,
            FOCUS_MODELS_STEMS,
            stems_box,
            &format!("Stem separation: {}", stem_token(stems)),
            "never / on-demand / always",
            true,
        ) {
            self.draft.local_runtimes.stem_separation = match stems {
                StemSeparation::Never => StemSeparation::OnDemand,
                StemSeparation::OnDemand => StemSeparation::Always,
                StemSeparation::Always => StemSeparation::Never,
            };
        }
        let doctor_box = UiRect::new(
            body.x + gpu_box.width + stems_box.width + 16.0,
            *cursor,
            150.0f32.min((body.width - gpu_box.width - stems_box.width - 16.0).max(0.0)),
            CONTROL_HEIGHT,
        );
        let busy = self.background.is_some();
        if self.picker(
            d,
            font,
            32,
            FOCUS_MODELS_DOCTOR,
            doctor_box,
            "Run doctor",
            "Runs tools/musializer_doctor.py --json and reads the runtime identities from it",
            !busy,
        ) {
            self.start_doctor();
        }
        *cursor += CONTROL_HEIGHT + 8.0;
    }

    /// The models-directory override field.
    ///
    /// Hand-drawn rather than a [`crate::ui::text_input::TextField`]: that widget
    /// reads raylib's keyboard directly, and the modal's whole keyboard contract
    /// is that this file owns every key while it is open.
    fn draw_path_field(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        boundary: UiRect,
    ) {
        if boundary.is_empty() {
            return;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, 33);
        let state = self.widgets.button(d, id, boundary);
        if state.clicked {
            self.focus = self
                .focus_order()
                .iter()
                .position(|control| *control == FOCUS_MODELS_DIR)
                .unwrap_or(self.focus);
        }
        let focused = self.is_focused(FOCUS_MODELS_DIR);
        self.models_dir_focused = focused;
        widgets::fill(d, boundary, color::ui_raised());
        d.draw_rectangle_lines_ex(
            widgets::rectangle(boundary),
            if focused { 2.0 } else { 1.0 },
            if focused {
                color::accent()
            } else {
                color::ui_rule()
            },
        );
        let text = if self.models_dir_field.is_empty() {
            "(unset \u{2014} the default above is used)".to_string()
        } else {
            self.models_dir_field.clone()
        };
        let tint = if self.models_dir_field.is_empty() {
            color::ui_muted()
        } else {
            color::ui_ink()
        };
        widgets::draw_text(
            d,
            font,
            &ellipsize(font, &text, boundary.width - 16.0, metric::UI_FONT_CAPTION),
            boundary.x + 8.0,
            boundary.y + (boundary.height - metric::UI_FONT_CAPTION) * 0.5,
            metric::UI_FONT_CAPTION,
            tint,
        );
        self.focus_ring(d, boundary, FOCUS_MODELS_DIR);
        let _ = self.take_activation(FOCUS_MODELS_DIR);
    }

    // -- 3. Codex ----------------------------------------------------------

    fn draw_codex(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let font = fonts.ui();
        heading(
            d,
            font,
            body,
            cursor,
            Section::Codex.heading(),
            "Codex runs the text-and-reasoning contracts over bounded JSON evidence. It is never \
             an audio timestamping engine, whatever its catalog advertises.",
        );

        subheading(d, font, body, cursor, "Executable");
        match &self.codex_binary {
            Some(path) => field_line(
                d,
                font,
                body,
                cursor,
                "codex",
                &path.display().to_string(),
                color::ui_success(),
            ),
            None => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "codex",
                    "not on PATH",
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "Install Codex and make sure `codex` is on this session's PATH. Musializer \
                     never reads Codex's authentication files.",
                    color::ui_muted(),
                );
            }
        }

        subheading(d, font, body, cursor, "Discovered models");
        let stamp = self
            .codex
            .document()
            .and_then(|cache| cache.fetched_at_utc.clone());
        let (state, age) = self.codex.badge(stamp.as_deref(), self.now);
        badge(
            d,
            font,
            UiRect::new(body.x, *cursor, 130.0f32.min(body.width), 18.0),
            state.token(),
            state.tint(),
        );
        widgets::draw_text(
            d,
            font,
            &age,
            body.x + 138.0,
            *cursor + 3.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        *cursor += 24.0;
        match self.codex.document() {
            None => {
                // §5 rule 6: exactly "Codex default", never a guessed id.
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Available",
                    CODEX_DEFAULT_LABEL,
                    color::ui_ink(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "Discovery has not run, or this Codex is too old to answer model/list. The \
                     only model offered is whatever Codex itself defaults to; Musializer will not \
                     guess an id.",
                    color::ui_muted(),
                );
            }
            Some(cache) => {
                let models = cache.models.clone();
                if models.is_empty() {
                    field_line(
                        d,
                        font,
                        body,
                        cursor,
                        "Available",
                        CODEX_DEFAULT_LABEL,
                        color::ui_ink(),
                    );
                }
                for model in models.iter().filter(|model| !model.hidden) {
                    let efforts: Vec<&str> = model
                        .supported_reasoning_efforts
                        .iter()
                        .map(|effort| effort.reasoning_effort.as_str())
                        .collect();
                    field_line(
                        d,
                        font,
                        body,
                        cursor,
                        &format!(
                            "{}{}",
                            model.id,
                            if model.is_default { " (default)" } else { "" }
                        ),
                        &format!(
                            "{} \u{00b7} efforts: {} \u{00b7} default: {}",
                            if model.display_name.is_empty() {
                                model.id.as_str()
                            } else {
                                model.display_name.as_str()
                            },
                            if efforts.is_empty() {
                                "not reported".to_string()
                            } else {
                                efforts.join("/")
                            },
                            model
                                .default_reasoning_effort
                                .as_deref()
                                .unwrap_or("not reported"),
                        ),
                        color::ui_ink(),
                    );
                }
            }
        }

        subheading(d, font, body, cursor, "Reasoning effort per eligible task");
        for (index, contract) in CODEX_ELIGIBLE.into_iter().enumerate() {
            let resolved = resolve_route(&self.draft, contract);
            let is_codex = resolved
                .route
                .as_ref()
                .is_some_and(|route| route.route_type == RouteType::Codex);
            let effort = resolved
                .route
                .as_ref()
                .and_then(|route| route.reasoning_effort);
            widgets::draw_text(
                d,
                font,
                contract.token(),
                body.x,
                *cursor + 6.0,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            let boundary = UiRect::new(
                body.x + 132.0,
                *cursor,
                180.0f32.min((body.width - 132.0).max(0.0)),
                CONTROL_HEIGHT,
            );
            let label = if is_codex {
                effort.map_or_else(
                    || "not set".to_string(),
                    |effort| effort_token(effort).to_string(),
                )
            } else {
                "not a Codex route".to_string()
            };
            if self.picker(
                d,
                font,
                600 + index as u32,
                FOCUS_CODEX_EFFORT_BASE + index as ControlId,
                boundary,
                &label,
                "minimal / low / medium / high",
                is_codex,
            ) {
                if let Some(mut route) = resolved.route.clone() {
                    route.reasoning_effort = Some(match effort {
                        None | Some(ReasoningEffort::High) => ReasoningEffort::Minimal,
                        Some(ReasoningEffort::Minimal) => ReasoningEffort::Low,
                        Some(ReasoningEffort::Low) => ReasoningEffort::Medium,
                        Some(ReasoningEffort::Medium) => ReasoningEffort::High,
                    });
                    set_override(&mut self.draft, route);
                }
            }
            *cursor += CONTROL_HEIGHT + ROW_GAP;
        }

        *cursor += 4.0;
        let busy = self.background.is_some();
        let refresh = UiRect::new(body.x, *cursor, 190.0f32.min(body.width), CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            610,
            FOCUS_CODEX_REFRESH,
            refresh,
            "Refresh Codex models",
            "Runs tools/codex_model_discovery.py, which starts a short-lived `codex app-server`",
            !busy && self.codex_binary.is_some(),
        ) {
            self.start_refresh(RefreshKind::Codex);
        }
        *cursor += CONTROL_HEIGHT + 8.0;
    }
}

/// A runtime's own sub-heading inside the Local models list.
fn subheading_row(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    title: &str,
) {
    *cursor += 4.0;
    widgets::draw_text(
        d,
        font,
        title,
        body.x,
        *cursor,
        metric::UI_FONT_CAPTION,
        color::ui_ink(),
    );
    *cursor += 19.0;
}

fn stem_token(value: StemSeparation) -> &'static str {
    match value {
        StemSeparation::Never => "never",
        StemSeparation::OnDemand => "on-demand",
        StemSeparation::Always => "always",
    }
}

fn effort_token(value: ReasoningEffort) -> &'static str {
    match value {
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
    }
}

impl AssistSettingsDialog {
    // -- 4. OpenRouter -----------------------------------------------------

    fn draw_openrouter(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let font = fonts.ui();
        heading(
            d, font, body, cursor,
            Section::OpenRouter.heading(),
            "The one remote provider. Its key lives in a 0600 file under your config directory \u{2014} \
             never in a .musi project, a preference file, a log, or a command line.",
        );

        subheading(d, font, body, cursor, "Connection");
        match self.credentials.clone() {
            CredentialState::None => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Key",
                    "none configured",
                    color::ui_muted(),
                );
            }
            CredentialState::Session { fingerprint } => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Key",
                    &format!("session only \u{00b7} {fingerprint}"),
                    color::ui_warning(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "Imported from OPENROUTER_API_KEY at startup and removed from this process's \
                     environment. It survives no restart.",
                    color::ui_muted(),
                );
            }
            CredentialState::File { fingerprint, label } => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Key",
                    &format!(
                        "stored \u{00b7} {fingerprint}{}",
                        label.map_or_else(String::new, |label| format!(" \u{00b7} {label}"))
                    ),
                    color::ui_success(),
                );
            }
            CredentialState::Refused { path, mode } => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Key",
                    &format!("refused \u{2014} mode {mode:04o}"),
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    &format!(
                        "{path} is readable by other users. Treat the credential as exposed, then \
                         run: chmod 600 {path}"
                    ),
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "Musializer refuses the file rather than repairing it: a silent chmod would \
                     hide the fact that the key has already been exposed.",
                    color::ui_muted(),
                );
            }
            CredentialState::Error(reason) => {
                field_line(d, font, body, cursor, "Key", &reason, color::ui_danger());
            }
        }

        subheading(d, font, body, cursor, "Replace");
        let field = UiRect::new(
            body.x,
            *cursor,
            (body.width - 8.0).max(80.0),
            CONTROL_HEIGHT + 4.0,
        );
        self.draw_key_field(d, font, field);
        *cursor += field.height + 6.0;
        paragraph(
            d,
            font,
            body,
            cursor,
            "Type or paste a key. Nothing you type here is drawn, measured or copyable \u{2014} the \
             field shows one bullet per character and nothing else.",
            color::ui_muted(),
        );
        *cursor += 4.0;

        let has_pending = self.pending_key.is_some();
        let busy = self.background.is_some();
        let button_width = 118.0f32.min((body.width - 24.0) / 4.0);
        let mut x = body.x;
        let test = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            700,
            FOCUS_KEY_TEST,
            test,
            "Test",
            "GET /api/v1/key \u{2014} read-only, non-inference, spends no credits",
            !busy && (has_pending || self.credentials.is_usable()),
        ) {
            self.start_key_test();
        }
        x += button_width + 8.0;
        let replace = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            701,
            FOCUS_KEY_REPLACE,
            replace,
            "Replace",
            "Commits the typed key. Enabled once a Test has succeeded.",
            !busy && has_pending && self.key_tested_ok,
        ) {
            self.commit_key();
        }
        x += button_width + 8.0;
        let untested = UiRect::new(x, *cursor, button_width + 30.0, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            702,
            FOCUS_KEY_SAVE_UNTESTED,
            untested,
            "Save untested",
            "Commits without a Test. The dialog will not claim the key works.",
            !busy && has_pending,
        ) {
            self.commit_key();
        }
        x += untested.width + 8.0;
        let forget = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            703,
            FOCUS_KEY_FORGET,
            forget,
            "Forget",
            "Removes this provider's entry and rewrites the file; other providers are untouched",
            !busy && matches!(self.credentials, CredentialState::File { .. }),
        ) {
            self.forget_key();
        }
        *cursor += CONTROL_HEIGHT + 8.0;
        if let Some(outcome) = self.key_test.clone() {
            paragraph(d, font, body, cursor, &outcome.sentence(), outcome.tint());
        }

        subheading(d, font, body, cursor, "Model catalog");
        let stamp = self
            .catalog
            .document()
            .and_then(|cache| cache.fetched_at_utc.clone());
        let (state, age) = self.catalog.badge(stamp.as_deref(), self.now);
        badge(
            d,
            font,
            UiRect::new(body.x, *cursor, 130.0f32.min(body.width), 18.0),
            state.token(),
            state.tint(),
        );
        widgets::draw_text(
            d,
            font,
            &age,
            body.x + 138.0,
            *cursor + 3.0,
            metric::UI_FONT_CAPTION,
            color::ui_muted(),
        );
        *cursor += 24.0;
        match &self.catalog {
            CacheSlot::NeverFetched => paragraph(
                d,
                font,
                body,
                cursor,
                "The catalog has never been fetched. This is not an empty catalog \u{2014} nothing \
                 has asked OpenRouter what it offers, so nothing is known.",
                color::ui_muted(),
            ),
            CacheSlot::Unreadable(reason) => {
                let reason = reason.clone();
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    &format!("The cached catalog could not be read: {reason}"),
                    color::ui_danger(),
                );
            }
            CacheSlot::Loaded(cache) => {
                let source = cache.source_url.clone();
                let filters: Vec<String> = cache
                    .filters
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect();
                let models = cache.models.clone();
                field_line(d, font, body, cursor, "Source", &source, color::ui_muted());
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Filters",
                    &if filters.is_empty() {
                        "none".to_string()
                    } else {
                        filters.join(", ")
                    },
                    color::ui_muted(),
                );
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Models",
                    &models.len().to_string(),
                    color::ui_ink(),
                );
                *cursor += 4.0;
                for model in models.iter().take(CATALOG_ROWS) {
                    let status = suitability::status(&model.id, ContractId::Semantic);
                    let modalities = format!(
                        "{} \u{2192} {}",
                        or_unreported(&model.input_modalities),
                        or_unreported(&model.output_modalities)
                    );
                    // Every one of these is echoed exactly as the cache holds it,
                    // and an absent field says "not reported" rather than being
                    // filled in from somewhere plausible.
                    let context = model.context_length.map_or_else(
                        || "context not reported".to_string(),
                        |length| format!("{length:.0} ctx"),
                    );
                    let price = match (&model.pricing.audio, &model.pricing.prompt, &model.pricing.completion) {
                        (Some(audio), _, _) => format!("audio ${audio}"),
                        (None, Some(prompt), Some(completion)) => {
                            format!("in ${prompt} / out ${completion}")
                        }
                        (None, Some(prompt), None) => format!("in ${prompt}"),
                        _ => "price not reported".to_string(),
                    };
                    field_line(
                        d,
                        font,
                        body,
                        cursor,
                        &model.id,
                        &format!(
                            "{} \u{00b7} {modalities} \u{00b7} {context} \u{00b7} {price} \u{00b7} {}",
                            if model.name.is_empty() {
                                "name not reported"
                            } else {
                                model.name.as_str()
                            },
                            status.token()
                        ),
                        if status == Suitability::Unsupported {
                            color::ui_danger()
                        } else {
                            color::ui_muted()
                        },
                    );
                }
                if models.len() > CATALOG_ROWS {
                    paragraph(
                        d,
                        font,
                        body,
                        cursor,
                        &format!("+{} more in the cache", models.len() - CATALOG_ROWS),
                        color::ui_muted(),
                    );
                }
            }
        }

        *cursor += 6.0;
        let network = self.draft.catalog.network_allowed;
        let refresh_on_open = self.draft.catalog.refresh_on_open;
        let toggle_width = 200.0f32.min((body.width - 16.0) / 3.0);
        let mut x = body.x;
        if self.toggle(
            d,
            font,
            710,
            FOCUS_CATALOG_NETWORK,
            UiRect::new(x, *cursor, toggle_width, CONTROL_HEIGHT),
            if network {
                "Network access: ON"
            } else {
                "Network access: off"
            },
            network,
            "Off means Refresh is the only thing that can open a socket, and it is disabled",
        ) {
            self.draft.catalog.network_allowed = !network;
        }
        x += toggle_width + 8.0;
        if self.toggle(
            d,
            font,
            711,
            FOCUS_CATALOG_REFRESH_ON_OPEN,
            UiRect::new(x, *cursor, toggle_width, CONTROL_HEIGHT),
            if refresh_on_open {
                "Refresh on open: ON"
            } else {
                "Refresh on open: off"
            },
            refresh_on_open,
            "Fetch a stale catalog when this section opens. Needs network access.",
        ) {
            self.draft.catalog.refresh_on_open = !refresh_on_open;
        }
        x += toggle_width + 8.0;
        if self.picker(
            d,
            font,
            712,
            FOCUS_CATALOG_REFRESH,
            UiRect::new(x, *cursor, toggle_width, CONTROL_HEIGHT),
            "Refresh now",
            "Runs tools/provider_catalog.py. Fetching a public catalog still discloses your IP \
             address, which is why it is never disguised as an offline action.",
            !busy && network,
        ) {
            self.start_refresh(RefreshKind::OpenRouter);
        }
        *cursor += CONTROL_HEIGHT + 6.0;
        if !network {
            paragraph(
                d,
                font,
                body,
                cursor,
                "Network access is off, so Refresh is disabled and the list above is whatever was \
                 last cached.",
                color::ui_muted(),
            );
        }
        *cursor += 4.0;
    }

    /// The masked Replace field.
    ///
    /// The one control in this application that must never draw what it holds.
    /// It draws [`masked_field`] of the buffer's **length**, so two different
    /// keys of the same length produce identical pixels — which is exactly what
    /// `tools/headless_check.sh` asserts by comparing two crops.
    fn draw_key_field(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        boundary: UiRect,
    ) {
        if boundary.is_empty() {
            return;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, 704);
        let state = self.widgets.button(d, id, boundary);
        if state.clicked {
            self.focus = self
                .focus_order()
                .iter()
                .position(|control| *control == FOCUS_KEY_FIELD)
                .unwrap_or(self.focus);
        }
        let focused = self.is_focused(FOCUS_KEY_FIELD);
        widgets::fill(d, boundary, color::ui_raised());
        d.draw_rectangle_lines_ex(
            widgets::rectangle(boundary),
            if focused { 2.0 } else { 1.0 },
            if focused {
                color::accent()
            } else {
                color::ui_rule()
            },
        );
        // Characters, not bytes: the sentence under the field promises one
        // bullet per character, and a pasted key need not be ASCII.
        let length = self
            .pending_key
            .as_ref()
            .map_or(0, |key| key.expose().chars().count());
        let drawn = masked_field(length);
        let text = if drawn.is_empty() {
            "paste or type a key \u{00b7} Ctrl+V".to_string()
        } else {
            drawn
        };
        let tint = if length == 0 {
            color::ui_muted()
        } else {
            color::ui_ink()
        };
        widgets::draw_text(
            d,
            font,
            &text,
            boundary.x + 10.0,
            boundary.y + (boundary.height - metric::UI_FONT_VALUE) * 0.5,
            metric::UI_FONT_VALUE,
            tint,
        );
        self.focus_ring(d, boundary, FOCUS_KEY_FIELD);
        let _ = self.take_activation(FOCUS_KEY_FIELD);
        self.widgets.hint(
            d,
            state,
            id,
            boundary,
            "Nothing typed here is drawn, measured or copyable",
        );
    }

    // -- 5. Privacy and diagnostics ----------------------------------------

    fn draw_privacy(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        fonts: &Faces,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let font = fonts.ui();
        heading(
            d, font, body, cursor,
            Section::Privacy.heading(),
            "What leaves this machine, where the configuration came from, and exactly which routes \
             the next job would use.",
        );

        subheading(d, font, body, cursor, "Remote audio policy");
        paragraph(
            d,
            font,
            body,
            cursor,
            "Audio can leave this machine only under TC-COARSE, TC-SEMANTIC and TC-VERIFY, and only \
             after a per-job confirmation that names the excerpts. TC-ALIGN is local-only by \
             contract even though remote aligners exist.",
            color::ui_muted(),
        );
        *cursor += 4.0;
        for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
            if contract.max_boundary().rank() < 2 {
                continue;
            }
            let resolved = resolve_route(&self.draft, contract);
            let zdr = resolved
                .route
                .as_ref()
                .and_then(|route| route.provider.as_ref())
                .is_some_and(|provider| provider.zdr_required);
            let is_remote = resolved
                .route
                .as_ref()
                .is_some_and(|route| route.route_type == RouteType::OpenRouter);
            widgets::draw_text(
                d,
                font,
                contract.token(),
                body.x,
                *cursor + 6.0,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            let boundary = UiRect::new(
                body.x + 132.0,
                *cursor,
                220.0f32.min((body.width - 132.0).max(0.0)),
                CONTROL_HEIGHT,
            );
            let tip = "Zero-data-retention endpoints only. If that leaves no eligible endpoint \
                       the request is refused rather than weakened.";
            // A contract resolved to a local route has no provider constraints
            // to set, so the control is *drawn disabled with its reason* rather
            // than drawn pressable and doing nothing. Missing beats pretending:
            // a control that silently ignores a press is worse than one that
            // explains itself.
            let pressed = if is_remote {
                self.toggle(
                    d,
                    font,
                    800 + index as u32,
                    FOCUS_PRIVACY_ZDR_BASE + index as ControlId,
                    boundary,
                    if zdr {
                        "ZDR required: ON"
                    } else {
                        "ZDR required: off"
                    },
                    zdr,
                    tip,
                )
            } else {
                self.picker(
                    d,
                    font,
                    800 + index as u32,
                    FOCUS_PRIVACY_ZDR_BASE + index as ControlId,
                    boundary,
                    "local route \u{00b7} nothing leaves",
                    tip,
                    false,
                )
            };
            if pressed {
                if let Some(mut route) = resolved.route.clone() {
                    let mut provider = route
                        .provider
                        .clone()
                        .unwrap_or_else(|| Provider::defaults_for(contract));
                    provider.zdr_required = !zdr;
                    route.provider = Some(provider);
                    set_override(&mut self.draft, route);
                }
            }
            *cursor += CONTROL_HEIGHT + ROW_GAP;
        }
        *cursor += 2.0;
        paragraph(
            d,
            font,
            body,
            cursor,
            "ZDR is on by default for every audio-leaving contract. A missing eligible endpoint \
             blocks the request and names the constraint; it never weakens it silently.",
            color::ui_muted(),
        );

        subheading(d, font, body, cursor, "Configuration provenance");
        field_line(
            d,
            font,
            body,
            cursor,
            "Settings file",
            &self.settings_path.as_ref().map_or_else(
                || "no config directory".to_string(),
                |path| path.display().to_string(),
            ),
            color::ui_ink(),
        );
        let (state_text, state_tint) = match &self.settings_source {
            SettingsSource::Loaded => ("loaded".to_string(), color::ui_success()),
            SettingsSource::Absent => (
                "no file yet \u{2014} the built-in defaults are in use".to_string(),
                color::ui_muted(),
            ),
            SettingsSource::Error(reason) => (reason.clone(), color::ui_danger()),
            SettingsSource::NoPath => ("no config directory".to_string(), color::ui_danger()),
        };
        field_line(d, font, body, cursor, "State", &state_text, state_tint);
        if let SettingsSource::Error(_) = &self.settings_source {
            paragraph(
                d,
                font,
                body,
                cursor,
                "The file is left exactly as it is. Saving would overwrite something you may be \
                 able to repair, so Save writes a new file only after you fix or move this one.",
                color::ui_danger(),
            );
        }
        field_line(
            d,
            font,
            body,
            cursor,
            "Schema",
            SETTINGS_SCHEMA,
            color::ui_muted(),
        );
        field_line(
            d,
            font,
            body,
            cursor,
            "Active profile",
            &self.draft.active_profile,
            color::ui_muted(),
        );
        field_line(
            d,
            font,
            body,
            cursor,
            "Credentials file",
            &self
                .credentials_path
                .as_ref()
                .map_or_else(|| "none".to_string(), |path| path.display().to_string()),
            color::ui_muted(),
        );
        field_line(
            d,
            font,
            body,
            cursor,
            "Credential state",
            &self.credentials.token(),
            if matches!(self.credentials, CredentialState::Refused { .. }) {
                color::ui_danger()
            } else {
                color::ui_muted()
            },
        );
        field_line(
            d,
            font,
            body,
            cursor,
            "Suitability overlay",
            suitability::OVERLAY_REVISION,
            color::ui_muted(),
        );
        let stamp = self
            .catalog
            .document()
            .and_then(|cache| cache.fetched_at_utc.clone());
        let (catalog_state, catalog_age) = self.catalog.badge(stamp.as_deref(), self.now);
        field_line(
            d,
            font,
            body,
            cursor,
            "Catalog refreshed",
            &format!("{catalog_age} ({})", catalog_state.token()),
            catalog_state.tint(),
        );

        subheading(
            d,
            font,
            body,
            cursor,
            "Dry run \u{2014} what the NEXT job would use",
        );
        paragraph(
            d,
            font,
            body,
            cursor,
            "Resolved from the settings above, not from a run. A job already in flight keeps the \
             route graph it snapshotted at Start.",
            color::ui_muted(),
        );
        *cursor += 4.0;
        for contract in ALL_CONTRACTS {
            let resolved = resolve_route(&self.draft, contract);
            let readiness = self.readiness(&resolved);
            field_line(
                d,
                font,
                body,
                cursor,
                // The resolved route's own contract, not the loop variable: if
                // the two ever disagreed the whole summary would be describing a
                // different task than it names.
                resolved.contract.token(),
                &format!(
                    "{} \u{00b7} {} \u{00b7} {} \u{00b7} fallback {} \u{00b7} {} \u{00b7} {}",
                    resolved.route_label(),
                    resolved.model_label(),
                    resolved.boundary().token(),
                    resolved
                        .route
                        .as_ref()
                        .map_or("\u{2014}", |route| route.fallback.token()),
                    resolved.origin.token(),
                    readiness.label(),
                ),
                readiness.tint(),
            );
        }

        *cursor += 6.0;
        let copy = UiRect::new(body.x, *cursor, 200.0f32.min(body.width), CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            810,
            FOCUS_PRIVACY_COPY,
            copy,
            "Copy diagnostics",
            "Puts the redacted summary above on the clipboard. It carries a fingerprint, never a key.",
            true,
        ) {
            let snapshot = self.describe().join("\n");
            let _ = d.set_clipboard_text(&snapshot);
            self.status = Some((
                "Redacted diagnostics copied to the clipboard.".to_string(),
                color::ui_success(),
            ));
        }
        *cursor += CONTROL_HEIGHT + 8.0;
    }
}

/// How many catalog rows the OpenRouter section lists before saying how many
/// more there are. A bound, so a 300-model catalog does not become a 300-row
/// scroll nobody reads.
const CATALOG_ROWS: usize = 12;

/// A modality list, or an honest "not reported" for an absent one.
fn or_unreported(values: &[String]) -> String {
    if values.is_empty() {
        "not reported".to_string()
    } else {
        values.join("+")
    }
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

impl AssistSettingsDialog {
    /// The modal's keyboard. **Every** key while the dialog is open belongs to
    /// this function: the shell's shortcut path is skipped in the same branch
    /// that calls it, and the timeline strip — the only surface with a widget
    /// that reads raylib's keyboard on its own — is not drawn at all.
    fn keyboard(&mut self, d: &mut RaylibDrawHandle<'_>) {
        use raylib::consts::KeyboardKey as Key;

        let control = d.is_key_down(Key::KEY_LEFT_CONTROL) || d.is_key_down(Key::KEY_RIGHT_CONTROL);
        let shift = d.is_key_down(Key::KEY_LEFT_SHIFT) || d.is_key_down(Key::KEY_RIGHT_SHIFT);
        let repeat = |d: &RaylibDrawHandle<'_>, key: Key| {
            d.is_key_pressed(key) || d.is_key_pressed_repeat(key)
        };

        if d.is_key_pressed(Key::KEY_ESCAPE) {
            self.escape();
            // Characters are drained either way, so a chord cannot leave one
            // queued for the next frame (`lyrics_editor_ui.c:204-207`).
            while d.get_char_pressed().is_some() {}
            return;
        }
        if repeat(d, Key::KEY_TAB) {
            self.focus_next(if shift { -1 } else { 1 });
        }
        if d.is_key_pressed(Key::KEY_ENTER) || d.is_key_pressed(Key::KEY_KP_ENTER) {
            self.activate = true;
        }

        let typing_key = self.is_focused(FOCUS_KEY_FIELD);
        let typing_path = self.is_focused(FOCUS_MODELS_DIR);

        if typing_key {
            if repeat(d, Key::KEY_BACKSPACE) {
                self.edit_key(|text| {
                    text.pop();
                });
            }
            if control && d.is_key_pressed(Key::KEY_V) {
                if let Ok(clipboard) = d.get_clipboard_text() {
                    let pasted: String = clipboard
                        .chars()
                        .filter(|character| !character.is_control())
                        .collect();
                    if !pasted.is_empty() {
                        self.edit_key(|text| text.push_str(&pasted));
                    }
                }
            }
        } else if typing_path {
            if repeat(d, Key::KEY_BACKSPACE) {
                self.models_dir_field.pop();
                self.sync_models_dir();
            }
            if control && d.is_key_pressed(Key::KEY_V) {
                if let Ok(clipboard) = d.get_clipboard_text() {
                    for character in clipboard.chars().filter(|c| !c.is_control()) {
                        self.models_dir_field.push(character);
                    }
                    self.sync_models_dir();
                }
            }
        }

        while let Some(codepoint) = d.get_char_pressed() {
            if control {
                continue;
            }
            if typing_key {
                self.edit_key(|text| text.push(codepoint));
            } else if typing_path && self.models_dir_field.len() < 512 {
                self.models_dir_field.push(codepoint);
                self.sync_models_dir();
            }
        }
    }

    /// Edits the pending credential in place.
    ///
    /// The buffer is unwrapped into a `String`, changed and re-wrapped, so the
    /// previous [`Secret`] drops — and zeroizes — on every keystroke. That is one
    /// transient copy per character, which is the best-effort claim §3 already
    /// makes: minimized scope and no durable plaintext copy, not that a managed
    /// runtime erased every copy.
    fn edit_key(&mut self, edit: impl FnOnce(&mut String)) {
        let mut text = self
            .pending_key
            .take()
            .map_or_else(String::new, |secret| secret.expose().to_string());
        edit(&mut text);
        // `String::truncate` panics off a char boundary, and a paste is not
        // guaranteed to be ASCII. The floor walks back to one.
        let cap = musializer_core::assist::secret::MAX_SECRET_BYTES;
        if text.len() > cap {
            let mut end = cap;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        // A key that changed is a key that has not been tested.
        self.key_tested_ok = false;
        self.key_test = None;
        self.pending_key = (!text.is_empty()).then(|| Secret::new(text));
    }

    fn sync_models_dir(&mut self) {
        self.draft.local_runtimes.models_dir = self.models_dir_field.trim().to_string();
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Which discovery cache a refresh rebuilds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshKind {
    OpenRouter,
    Codex,
}

impl RefreshKind {
    fn tool(self) -> &'static str {
        match self {
            RefreshKind::OpenRouter => "provider_catalog.py",
            RefreshKind::Codex => "codex_model_discovery.py",
        }
    }
}

impl AssistSettingsDialog {
    /// Writes `assist.json`.
    ///
    /// A corrupt existing file is **not** overwritten on the first press: §2 says
    /// a corrupt file is an error rather than a silent reset, and the whole point
    /// of refusing it at load is that the user may be able to repair it. So the
    /// first Save over one arms an explicit second press instead.
    fn save(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            self.status = Some((
                "No per-user configuration directory could be resolved, so there is nowhere to \
                 save. Set XDG_CONFIG_HOME or HOME."
                    .to_string(),
                color::ui_danger(),
            ));
            return;
        };
        if let SettingsSource::Error(reason) = self.settings_source.clone() {
            if !self.overwrite_armed {
                self.overwrite_armed = true;
                self.status = Some((
                    format!(
                        "{reason} \u{2014} press Save again to replace it, or repair it first."
                    ),
                    color::ui_danger(),
                ));
                return;
            }
        }
        match files::save_settings(&path, &self.draft) {
            Ok(()) => {
                self.saved = self.draft.clone();
                self.settings_source = SettingsSource::Loaded;
                self.overwrite_armed = false;
                self.confirm_close = false;
                // The resolved models directory can change with the override, so
                // it is re-resolved rather than left showing the old answer.
                if let Ok(resolution) = models::resolve(Some(&self.draft)) {
                    self.models_dir = Some(resolution);
                    self.models_dir_error = None;
                }
                self.status = Some((
                    format!(
                        "Saved to {}. These routes apply to the next job.",
                        path.display()
                    ),
                    color::ui_success(),
                ));
            }
            Err(error) => {
                self.status = Some((format!("Could not save: {error}"), color::ui_danger()));
            }
        }
    }

    /// `Test` (§3): `GET /api/v1/key`, read-only and non-inference.
    ///
    /// Only ever reached from an explicit press or an Enter on the focused
    /// button. [`PROBE_KEY_TEST_VARIABLE`] short-circuits it, which is how a
    /// capture run photographs every outcome without opening a socket.
    fn start_key_test(&mut self) {
        if self.background.is_some() {
            return;
        }
        if let Ok(stub) = std::env::var(PROBE_KEY_TEST_VARIABLE) {
            match KeyTest::parse(&stub) {
                Some(outcome) => {
                    self.key_tested_ok = matches!(outcome, KeyTest::Ok { .. });
                    self.key_test = Some(outcome);
                }
                None => {
                    self.key_test = Some(KeyTest::NoNetwork(format!(
                        "{PROBE_KEY_TEST_VARIABLE} names no outcome: {stub:?}"
                    )));
                    self.key_tested_ok = false;
                }
            }
            return;
        }
        // The pending key first: testing what the user just typed is the point of
        // the Replace flow. Otherwise the stored one, read at this moment rather
        // than held since the dialog opened.
        let (secret, from_pending) = match self.pending_key.take() {
            Some(secret) => (Some(secret), true),
            None => (self.stored_secret(), false),
        };
        let Some(secret) = secret else {
            self.key_test = Some(KeyTest::NoKey);
            self.key_tested_ok = false;
            return;
        };
        self.test_returns_to_pending = from_pending;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(run_key_test(secret));
        });
        self.background = Some(Background::KeyTest(receiver));
        self.key_test = None;
    }

    /// Reads the stored credential, for a `Test` of a key already on disk.
    fn stored_secret(&self) -> Option<Secret> {
        let path = self.credentials_path.as_ref()?;
        let store = files::load_credentials(path).ok()??;
        let entry = store.get("openrouter", credential_lookup(&self.draft))?;
        Some(Secret::new(entry.secret.expose().to_string()))
    }

    /// `Replace` / `Save untested` (§3).
    fn commit_key(&mut self) {
        let Some(path) = self.credentials_path.clone() else {
            self.status = Some((
                "No per-user configuration directory, so there is nowhere to store a key."
                    .to_string(),
                color::ui_danger(),
            ));
            return;
        };
        if let CredentialState::Refused { path, mode } = &self.credentials {
            self.status = Some((
                format!(
                    "{path} is mode {mode:04o}. Fix it first (chmod 600) \u{2014} writing over it \
                     would hide that the existing key was exposed."
                ),
                color::ui_danger(),
            ));
            return;
        }
        let Some(secret) = self.pending_key.take() else {
            self.status = Some(("There is no key to store.".to_string(), color::ui_muted()));
            return;
        };
        let fingerprint = secret.fingerprint();
        let label = match &self.key_test {
            Some(KeyTest::Ok { label, .. }) => label.clone(),
            _ => None,
        };
        let mut store = match files::load_credentials(&path) {
            Ok(Some(store)) => store,
            Ok(None) => musializer_core::assist::credentials::CredentialStore::new(),
            Err(error) => {
                self.status = Some((
                    format!("Could not read the credentials file: {error}"),
                    color::ui_danger(),
                ));
                return;
            }
        };
        let lookup = credential_lookup(&self.draft).to_string();
        if !store.insert("openrouter", &lookup, secret, label.clone()) {
            self.status = Some((
                "The credentials file would not accept this account label.".to_string(),
                color::ui_danger(),
            ));
            return;
        }
        match files::save_credentials(&path, &store) {
            Ok(()) => {
                self.draft.credentials.openrouter.mode = CredentialMode::File;
                self.draft.credentials.openrouter.lookup_id = lookup;
                self.draft.credentials.openrouter.fingerprint = Some(fingerprint.clone());
                self.draft.credentials.openrouter.label = label.clone();
                self.credentials = CredentialState::File { fingerprint, label };
                self.key_tested_ok = false;
                self.status = Some((
                    format!(
                        "Key stored in {} (mode 0600). Save to record its fingerprint in the \
                         settings file.",
                        path.display()
                    ),
                    color::ui_success(),
                ));
            }
            Err(error) => {
                self.status = Some((
                    format!("Could not write the credentials file: {error}"),
                    color::ui_danger(),
                ));
            }
        }
    }

    /// `Forget` (§3): one provider's entry goes, everyone else's bytes stay
    /// identical, and an emptied file is removed rather than left as an empty
    /// shell.
    fn forget_key(&mut self) {
        let Some(path) = self.credentials_path.clone() else {
            return;
        };
        let lookup = credential_lookup(&self.draft).to_string();
        match files::forget_credential(&path, "openrouter", &lookup) {
            Ok(true) => {
                self.credentials = CredentialState::None;
                self.draft.credentials.openrouter =
                    musializer_core::assist::settings::CredentialRecord::default();
                self.key_test = None;
                self.key_tested_ok = false;
                self.status = Some((
                    "Key forgotten. Other providers' entries were left byte-identical.".to_string(),
                    color::ui_success(),
                ));
            }
            Ok(false) => {
                self.status = Some((
                    "There was no stored key to forget.".to_string(),
                    color::ui_muted(),
                ));
            }
            Err(error) => {
                self.status = Some((format!("Could not forget: {error}"), color::ui_danger()));
            }
        }
    }

    /// Rebuilds one discovery cache through its Python tool, off the frame
    /// thread.
    fn start_refresh(&mut self, kind: RefreshKind) {
        if self.background.is_some() {
            return;
        }
        if let Ok(stub) = std::env::var(PROBE_REFRESH_VARIABLE) {
            self.status = Some(match stub.trim() {
                "ok" => (
                    "Catalog refresh stubbed: the cache on disk is unchanged.".to_string(),
                    color::ui_success(),
                ),
                other => (
                    format!("Catalog refresh stubbed as a failure: {other}"),
                    color::ui_warning(),
                ),
            });
            return;
        }
        let Some(tool) = find_tool(kind.tool()) else {
            self.status = Some((
                format!(
                    "{} is missing from this installation, so discovery cannot run.",
                    kind.tool()
                ),
                color::ui_danger(),
            ));
            return;
        };
        let mut command = Command::new("python3");
        command.arg(&tool);
        match kind {
            RefreshKind::OpenRouter => {
                command.arg("refresh");
                let filters = if self.draft.catalog.last_filters.is_empty() {
                    vec![
                        ("input_modalities".to_string(), "audio".to_string()),
                        ("output_modalities".to_string(), "text".to_string()),
                    ]
                } else {
                    self.draft
                        .catalog
                        .last_filters
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                };
                for (key, value) in filters {
                    command.arg("--filter").arg(format!("{key}={value}"));
                }
            }
            RefreshKind::Codex => {
                command.arg("refresh");
            }
        }
        // The same rule `external_analysis.py::_safe_local_env` applies to its own
        // children (E1): nothing credential-shaped travels to a process that does
        // not need it, and neither of these tools needs one — the OpenRouter
        // catalog endpoint is unauthenticated and Codex authenticates itself.
        strip_credential_variables(&mut command);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = match command.output() {
                Ok(output) if output.status.success() => Ok(()),
                Ok(output) => Err(String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("the discovery tool failed")
                    .to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(outcome);
        });
        self.background = Some(Background::Refresh(receiver));
    }

    /// Runs the doctor for the runtime identities the Local models section shows.
    fn start_doctor(&mut self) {
        if self.background.is_some() {
            return;
        }
        let Some(tool) = find_tool("musializer_doctor.py") else {
            self.doctor_error =
                Some("tools/musializer_doctor.py is missing from this installation".to_string());
            return;
        };
        let mut command = Command::new("python3");
        command.arg(&tool).arg("--json");
        strip_credential_variables(&mut command);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = match command.output() {
                Ok(output) => serde_json::from_slice::<DoctorReport>(&output.stdout)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(outcome);
        });
        self.background = Some(Background::Doctor(receiver));
    }

    /// Collects whatever a background job finished. Never blocks: a settings
    /// dialog that stalled the frame on a network timeout would be the hang the
    /// stale/offline rule exists to avoid.
    fn poll_background(&mut self, d: &mut RaylibDrawHandle<'_>) {
        let _ = d;
        let Some(job) = &self.background else {
            return;
        };
        enum Finished {
            KeyTest(KeyTest, Secret),
            Refresh(Result<(), String>),
            Doctor(Result<DoctorReport, String>),
        }
        let finished = match job {
            Background::KeyTest(receiver) => match receiver.try_recv() {
                Ok((outcome, secret)) => Some(Finished::KeyTest(outcome, secret)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Finished::KeyTest(
                    KeyTest::NoNetwork("the check ended without an answer".to_string()),
                    Secret::new(String::new()),
                )),
            },
            Background::Refresh(receiver) => match receiver.try_recv() {
                Ok(result) => Some(Finished::Refresh(result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Finished::Refresh(Err(
                    "the discovery tool ended without an answer".to_string(),
                ))),
            },
            Background::Doctor(receiver) => match receiver.try_recv() {
                Ok(result) => Some(Finished::Doctor(result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Finished::Doctor(Err(
                    "the doctor ended without an answer".to_string(),
                ))),
            },
        };
        let Some(finished) = finished else {
            return;
        };
        self.background = None;
        match finished {
            Finished::KeyTest(outcome, secret) => {
                self.key_tested_ok = matches!(outcome, KeyTest::Ok { .. });
                if self.test_returns_to_pending && !secret.is_empty() {
                    self.pending_key = Some(secret);
                }
                self.key_test = Some(outcome);
            }
            Finished::Refresh(Ok(())) => {
                self.catalog =
                    read_cache("openrouter-models-v1.json", |document: &CatalogCache| {
                        document.schema_version == "musializer.openrouter-catalog/v1"
                    });
                self.codex = read_cache("codex-models-v1.json", |document: &CodexCache| {
                    document.schema_version == "musializer.codex-model-catalog/v1"
                });
                self.now = now_utc();
                self.status = Some((
                    "Discovery cache refreshed.".to_string(),
                    color::ui_success(),
                ));
            }
            Finished::Refresh(Err(reason)) => {
                // The prior valid cache stays readable, which is the whole point
                // of the tools' atomic replacement — so the list above is still
                // the last good one rather than empty.
                self.status = Some((
                    format!("Refresh failed: {reason}. The last valid cache is still shown."),
                    color::ui_warning(),
                ));
            }
            Finished::Doctor(Ok(report)) => {
                self.doctor = Some(report);
                self.doctor_error = None;
                self.status = Some((
                    "Doctor finished; runtime identities updated.".to_string(),
                    color::ui_success(),
                ));
            }
            Finished::Doctor(Err(reason)) => {
                self.doctor = None;
                self.doctor_error = Some(reason);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syntactically valid key with a unique sentinel, the same shape
    /// `tools/secret_canary_check.sh` plants.
    const CANARY: &str = "sk-or-v1-MUSICANARY7Q4X2ZK9aaaabbbbcccc";

    fn dialog() -> AssistSettingsDialog {
        let mut dialog = AssistSettingsDialog::new();
        dialog.open = true;
        dialog
    }

    // -- the masked field --------------------------------------------------

    /// The property the capture assertion rests on: the drawn string is a
    /// function of the **length** alone, so two different keys of the same
    /// length are indistinguishable on screen.
    #[test]
    fn the_masked_field_is_a_function_of_length_alone() {
        let a = "A".repeat(20);
        let b = "W".repeat(20);
        assert_eq!(masked_field(a.len()), masked_field(b.len()));
        assert_eq!(masked_field(20), "\u{2022}".repeat(20));
        assert_eq!(masked_field(0), "");
    }

    /// Not one character of the key, at any length, including past the cap.
    #[test]
    fn the_masked_field_never_contains_a_character_of_the_key() {
        for length in [1usize, 8, 20, MASK_CAP, MASK_CAP + 1, 200] {
            let drawn = masked_field(length);
            for character in CANARY.chars() {
                assert!(
                    !drawn.contains(character),
                    "the mask for {length} characters drew {character:?}"
                );
            }
            assert!(drawn.chars().count() <= MASK_CAP + 1);
        }
        // And past the cap the mask stops reporting the length exactly.
        assert_eq!(masked_field(MASK_CAP + 1), masked_field(4096));
    }

    /// The report line is the other place a key could escape.
    #[test]
    fn no_report_line_can_contain_the_key() {
        let mut dialog = dialog();
        dialog.pending_key = Some(Secret::new(CANARY.to_string()));
        let lines = dialog.describe().join("\n");
        assert!(!lines.contains("MUSICANARY"));
        assert!(!lines.contains("sk-or"));
        assert!(lines.contains("masked=\"\u{2022}"));
        // The fingerprint is the one identity that may appear, and it is not any
        // part of the key.
        let fingerprint = Secret::new(CANARY.to_string()).fingerprint();
        assert_eq!(fingerprint.len(), 8);
        assert!(!CANARY.contains(&fingerprint));
    }

    #[test]
    fn a_debug_print_of_the_pending_key_is_redacted() {
        let secret = Secret::new(CANARY.to_string());
        assert!(!format!("{secret:?}").contains("MUSICANARY"));
    }

    // -- layout ------------------------------------------------------------

    /// The content region has to stay inside the dialog at every supported
    /// window, or a section body would draw over the footer. A capture cannot
    /// catch this: everything moves together and still looks self-coherent.
    #[test]
    fn the_content_region_stays_inside_the_dialog_at_every_supported_window() {
        for window in [
            (960.0f32, 640.0f32),
            (1280.0, 720.0),
            (1600.0, 900.0),
            (2560.0, 1440.0),
            (800.0, 640.0),
            (1024.0, 600.0),
        ] {
            let layout = DialogLayout::of(window);
            assert!(
                layout.dialog.contains(layout.content),
                "{window:?}: content {:?} escapes {:?}",
                layout.content,
                layout.dialog
            );
            assert!(layout.dialog.contains(layout.header));
            assert!(layout.dialog.contains(layout.footer));
            assert!(layout.dialog.contains(layout.close));
            assert!(layout.dialog.contains(layout.save));
            assert!(!layout.body().is_empty(), "{window:?} has no body");
            for tab in layout.tabs {
                assert!(layout.dialog.contains(tab), "{window:?}: tab escapes");
            }
            // The dialog never exceeds the window, so the scrim is never the
            // only thing keeping it on screen.
            assert!(layout.dialog.x >= 0.0 && layout.dialog.y >= 0.0);
            assert!(layout.dialog.x + layout.dialog.width <= window.0 + 0.5);
            assert!(layout.dialog.y + layout.dialog.height <= window.1 + 0.5);
        }
    }

    /// The rail is shed once, at one width, and never comes back as the window
    /// narrows. The transport row's width sweep is why this is asserted as
    /// monotonicity rather than as "the rail goes at 790".
    #[test]
    fn the_rail_is_shed_monotonically_in_width() {
        let mut seen_tabs = false;
        let mut width = 1600.0f32;
        while width >= 640.0 {
            let layout = DialogLayout::of((width, 800.0));
            if layout.rail_on_left {
                assert!(!seen_tabs, "the rail came back at {width} after being shed");
            } else {
                seen_tabs = true;
            }
            width -= 4.0;
        }
        assert!(seen_tabs, "the rail was never shed");
        assert!(DialogLayout::of((1280.0, 720.0)).rail_on_left);
        assert!(!DialogLayout::of((760.0, 640.0)).rail_on_left);
    }

    /// The routing matrix stacks rather than clipping, and its columns always
    /// sum to the body width.
    #[test]
    fn the_routing_columns_fill_the_body_without_overflowing_it() {
        for width in [ROUTING_MATRIX_BODY_WIDTH, 700.0, 860.0, 1010.0] {
            let columns = AssistSettingsDialog::routing_columns(width);
            let total: f32 = columns.iter().sum::<f32>() + 6.0 * 5.0;
            assert!(
                (total - width).abs() < 0.01,
                "{width}: columns total {total}"
            );
            assert!(columns.iter().all(|column| *column >= 90.0));
        }
        // Every layout that keeps the matrix has at least the body the matrix
        // needs, which is the link between the two constants.
        let mut width = 2560.0f32;
        while width >= 640.0 {
            let layout = DialogLayout::of((width, 900.0));
            if !layout.routing_compact() {
                assert!(
                    layout.body().width >= ROUTING_MATRIX_BODY_WIDTH,
                    "{width}: the matrix drew into a {} px body",
                    layout.body().width
                );
            }
            width -= 2.0;
        }
    }

    /// The matrix is shed once and never comes back, which is the property the
    /// threshold is written in dialog widths to buy. Measured, because deciding
    /// from the body width passes every static check and still oscillates.
    #[test]
    fn the_routing_matrix_is_shed_monotonically_in_width() {
        let mut seen_compact = false;
        let mut width = 2560.0f32;
        while width >= 640.0 {
            let compact = DialogLayout::of((width, 900.0)).routing_compact();
            if compact {
                seen_compact = true;
            } else {
                assert!(!seen_compact, "the matrix came back at {width}");
            }
            width -= 2.0;
        }
        assert!(seen_compact, "the matrix never stacked");
        assert!(!DialogLayout::of((1280.0, 720.0)).routing_compact());
        // 1280 px at 150% is 853 logical, which is the scaled capture's layout.
        assert!(DialogLayout::of((853.0, 480.0)).routing_compact());
    }

    // -- focus -------------------------------------------------------------

    /// Tab walks a closed ring: every step lands on a control, the ring wraps,
    /// and Shift-Tab is its exact inverse.
    #[test]
    fn tab_walks_a_closed_ring_and_shift_tab_undoes_it() {
        for section in Section::ALL {
            let mut dialog = dialog();
            dialog.section = section;
            let total = dialog.focus_order().len();
            assert!(total >= 8, "{section:?} offers only {total} controls");
            let start = dialog.focused();
            for _ in 0..total {
                dialog.focus_next(1);
                assert!(dialog.focused().is_some());
            }
            assert_eq!(dialog.focused(), start, "{section:?} did not wrap");
            for _ in 0..total {
                dialog.focus_next(-1);
            }
            assert_eq!(dialog.focused(), start);
            // Backwards from the first control lands on the last, not nowhere.
            dialog.focus = 0;
            dialog.focus_next(-1);
            assert_eq!(dialog.focus_position().0, total - 1);
        }
    }

    /// No control id appears twice in a ring: a duplicate would make one Enter
    /// activate two controls, and the ring would be a lie about where the focus
    /// is.
    #[test]
    fn a_focus_ring_never_names_the_same_control_twice() {
        for section in Section::ALL {
            let mut dialog = dialog();
            dialog.section = section;
            let order = dialog.focus_order();
            let unique: std::collections::BTreeSet<ControlId> = order.iter().copied().collect();
            assert_eq!(unique.len(), order.len(), "{section:?} has a duplicate");
        }
    }

    /// One Enter activates exactly one control.
    #[test]
    fn an_activation_is_consumed_by_the_focused_control_only() {
        let mut dialog = dialog();
        dialog.section = Section::Routing;
        dialog.focus = 0;
        let focused = dialog.focused().unwrap();
        dialog.activate = true;
        assert!(!dialog.take_activation(focused + 9999));
        assert!(dialog.take_activation(focused));
        assert!(!dialog.take_activation(focused), "one Enter fired twice");
    }

    /// The locked contract has no pickers, so Tab cannot land on one.
    #[test]
    fn the_locked_contract_offers_no_focusable_control() {
        let dialog = dialog();
        let order = dialog.focus_order();
        let locked = ALL_CONTRACTS
            .iter()
            .position(|contract| *contract == ContractId::Measured)
            .unwrap() as ControlId;
        for base in [FOCUS_ROUTE_BASE, FOCUS_MODEL_BASE, FOCUS_FALLBACK_BASE] {
            assert!(!order.contains(&(base + locked)));
        }
        assert!(ContractId::Measured.is_locked());
    }

    // -- escape and unsaved edits -----------------------------------------

    #[test]
    fn escape_closes_a_clean_dialog_immediately() {
        let mut dialog = dialog();
        assert!(!dialog.is_dirty());
        dialog.escape();
        assert!(!dialog.is_open());
    }

    /// One explicit confirm step, no silent discard.
    #[test]
    fn escape_with_unsaved_edits_asks_before_discarding() {
        let mut dialog = dialog();
        dialog.draft.catalog.show_experimental = true;
        assert!(dialog.is_dirty());
        dialog.escape();
        assert!(dialog.is_open(), "the first Escape discarded silently");
        assert!(dialog.confirm_close);
        dialog.escape();
        assert!(!dialog.is_open());
    }

    /// Going back to editing cancels the pending discard, so a stale "Escape
    /// again" cannot throw away work typed after it.
    #[test]
    fn traversing_cancels_a_pending_discard() {
        let mut dialog = dialog();
        dialog.draft.catalog.show_experimental = true;
        dialog.escape();
        assert!(dialog.confirm_close);
        dialog.focus_next(1);
        assert!(!dialog.confirm_close);
        dialog.escape();
        assert!(dialog.is_open(), "the discard was not re-armed");
    }

    // -- routing model -----------------------------------------------------

    /// Every built-in recommended route is legal against the contract table —
    /// which is the same validation `assist.json` applies, so a profile written
    /// from these defaults round-trips.
    #[test]
    fn every_recommended_route_satisfies_its_own_contract() {
        for contract in ALL_CONTRACTS {
            let Some(route) = recommended_route(contract) else {
                assert_eq!(contract, ContractId::Verify, "{contract:?} has no default");
                continue;
            };
            assert_eq!(route.contract, contract);
            assert!(contract.eligible_route_types().contains(&route.route_type));
            assert!(contract.allowed_fallbacks().contains(&route.fallback));
            assert!(route.route_type.minimum_boundary().rank() <= contract.max_boundary().rank());
            assert_eq!(
                route.reasoning_effort.is_some(),
                route.route_type == RouteType::Codex
            );
            assert_eq!(
                route.provider.is_some(),
                route.route_type == RouteType::OpenRouter
            );
        }
    }

    /// The recommended aligner is the one the benchmark measured, and the one it
    /// failed is never offered.
    #[test]
    fn the_recommended_aligner_is_the_benchmarked_one() {
        let route = recommended_route(ContractId::Align).unwrap();
        assert_eq!(route.model_id.as_deref(), Some("mms-ctc"));
        assert_eq!(
            suitability::status("mms-ctc", ContractId::Align),
            Suitability::Recommended
        );
        let offered = model_options(
            ContractId::Align,
            RouteType::LocalProc,
            None,
            None,
            true,
            None,
        );
        assert!(offered.contains(&"mms-ctc".to_string()));
        assert!(
            !offered.contains(&"qwen3-fa".to_string()),
            "an unsupported model was offered with experimental on"
        );
    }

    /// `unsupported` is never offered, `experimental` needs asking, and the
    /// selected value is never dropped from its own picker.
    #[test]
    fn the_picker_honours_the_suitability_overlay() {
        let catalog = CatalogCache {
            schema_version: "musializer.openrouter-catalog/v1".to_string(),
            source_url: String::new(),
            fetched_at_utc: None,
            filters: BTreeMap::new(),
            models: vec![
                CatalogModel {
                    id: "xiaomi/mimo-v2.5".to_string(),
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["text".to_string()],
                    ..CatalogModel::default()
                },
                CatalogModel {
                    id: "acme/never-measured".to_string(),
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["text".to_string()],
                    ..CatalogModel::default()
                },
                CatalogModel {
                    id: "acme/text-only".to_string(),
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                    ..CatalogModel::default()
                },
            ],
        };
        let hidden = model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            false,
            None,
        );
        assert!(
            hidden.is_empty(),
            "experimental models were offered: {hidden:?}"
        );
        let shown = model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            true,
            None,
        );
        assert_eq!(
            shown,
            vec![
                "xiaomi/mimo-v2.5".to_string(),
                "acme/never-measured".to_string()
            ],
            "a text-only model was offered as an audio route"
        );
        // A selected model the filter would drop stays in its own picker.
        let with_selected = model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            false,
            Some("xiaomi/mimo-v2.5"),
        );
        assert_eq!(with_selected, vec!["xiaomi/mimo-v2.5".to_string()]);
    }

    /// Absent Codex discovery offers exactly `Codex default` — never a guessed
    /// id (§5 rule 6).
    #[test]
    fn absent_codex_discovery_offers_exactly_the_default() {
        let options = model_options(
            ContractId::Wording,
            RouteType::Codex,
            None,
            None,
            true,
            None,
        );
        assert_eq!(options, vec![CODEX_DEFAULT_LABEL.to_string()]);
        let resolved = resolve_route(&AssistSettings::default(), ContractId::Wording);
        assert_eq!(resolved.model_label(), CODEX_DEFAULT_LABEL);
        assert!(resolved
            .route
            .as_ref()
            .is_some_and(|route| route.model_id.is_none()));
    }

    /// An override is stored under the `custom` profile, and cycling back to the
    /// recommended value removes the entry rather than pinning today's default
    /// forever.
    #[test]
    fn an_override_is_stored_and_then_removed_when_it_matches_the_default_again() {
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.fallback = FallbackPolicy::Ask;
        set_override(&mut settings, route.clone());
        assert_eq!(settings.active_profile, OVERRIDE_PROFILE_ID);
        assert_eq!(
            resolve_route(&settings, ContractId::Coarse).origin,
            RouteOrigin::Override
        );
        settings.validate().expect("an override must be storable");

        set_override(
            &mut settings,
            recommended_route(ContractId::Coarse).unwrap(),
        );
        assert_eq!(
            resolve_route(&settings, ContractId::Coarse).origin,
            RouteOrigin::Recommended
        );
        assert!(settings
            .profile(OVERRIDE_PROFILE_ID)
            .is_some_and(|profile| profile.routes.is_empty()));
    }

    /// A whole profile of overrides survives a write and a read, which is the
    /// only thing that makes the dialog's Save meaningful.
    #[test]
    fn a_dialog_written_profile_round_trips_through_the_settings_schema() {
        let mut settings = AssistSettings::default();
        for contract in ALL_CONTRACTS {
            if contract.is_locked() {
                continue;
            }
            if let Some(mut route) = recommended_route(contract) {
                route.fallback = *contract.allowed_fallbacks().last().unwrap();
                set_override(&mut settings, route);
            }
        }
        let bytes = settings.to_bytes().expect("the draft must serialize");
        let parsed = AssistSettings::parse(&bytes).expect("and parse back");
        assert_eq!(parsed, settings);
        assert!(!String::from_utf8(bytes).unwrap().contains("secret"));
    }

    /// The boundary shown is the route's own, never the contract's ceiling —
    /// telling a user a local Whisper lane sends audio off the machine would be
    /// the worst kind of wrong.
    #[test]
    fn the_displayed_boundary_is_the_routes_own() {
        let settings = AssistSettings::default();
        let coarse = resolve_route(&settings, ContractId::Coarse);
        assert_eq!(
            ContractId::Coarse.max_boundary(),
            Boundary::AudioLeavesMachine
        );
        assert_eq!(coarse.boundary(), Boundary::LocalOnly);
        // A remote route reaches its contract's ceiling: TC-SEMANTIC sends the
        // track, so `text-leaves-machine` would understate it.
        let semantic = resolve_route(&settings, ContractId::Semantic);
        assert_eq!(semantic.boundary(), Boundary::AudioLeavesMachine);
        assert_eq!(
            resolve_route(&settings, ContractId::Wording).boundary(),
            Boundary::TextLeavesMachine
        );
        assert_eq!(
            resolve_route(&settings, ContractId::Verify).boundary(),
            Boundary::LocalOnly,
            "an unrouted contract cannot claim data leaves the machine"
        );
        // And no resolved route may ever exceed its own contract.
        for contract in ALL_CONTRACTS {
            let resolved = resolve_route(&settings, contract);
            assert!(resolved.boundary().rank() <= contract.max_boundary().rank());
        }
    }

    /// `TC-VERIFY` has no measured default, and the dialog says so instead of
    /// inventing one.
    #[test]
    fn an_unrouted_contract_says_so_rather_than_guessing() {
        let resolved = resolve_route(&AssistSettings::default(), ContractId::Verify);
        assert_eq!(resolved.origin, RouteOrigin::Unrouted);
        assert!(resolved.route.is_none());
        assert_eq!(resolved.route_label(), "no route");
        assert_eq!(resolved.model_label(), "\u{2014}");
        let dialog = dialog();
        assert!(matches!(dialog.readiness(&resolved), Readiness::Blocked(_)));
    }

    // -- readiness ---------------------------------------------------------

    /// "Not probed" and "missing" are different sentences, and only one of them
    /// is a claim about the machine.
    #[test]
    fn an_unprobed_runtime_is_unknown_rather_than_blocked() {
        let mut dialog = dialog();
        let align = resolve_route(&dialog.draft, ContractId::Align);
        assert!(matches!(dialog.readiness(&align), Readiness::Unknown(_)));

        dialog.doctor = Some(DoctorReport {
            schema_version: String::new(),
            runtimes: BTreeMap::from([(
                "mms_ctc_aligner".to_string(),
                RuntimeIdentity {
                    state: "unavailable".to_string(),
                    remediation: Some("install the alignment venv".to_string()),
                    ..RuntimeIdentity::default()
                },
            )]),
        });
        match dialog.readiness(&align) {
            Readiness::Blocked(reason) => assert_eq!(reason, "install the alignment venv"),
            other => panic!("expected a blocked route, got {other:?}"),
        }

        dialog.doctor = Some(DoctorReport {
            schema_version: String::new(),
            runtimes: BTreeMap::from([(
                "mms_ctc_aligner".to_string(),
                RuntimeIdentity {
                    state: "available".to_string(),
                    ..RuntimeIdentity::default()
                },
            )]),
        });
        assert_eq!(dialog.readiness(&align), Readiness::Ready);
    }

    #[test]
    fn an_openrouter_route_without_a_key_is_blocked_by_name() {
        let dialog = dialog();
        let semantic = resolve_route(&dialog.draft, ContractId::Semantic);
        match dialog.readiness(&semantic) {
            Readiness::Blocked(reason) => assert_eq!(reason, "No key"),
            other => panic!("expected blocked, got {other:?}"),
        }
    }

    // -- caches ------------------------------------------------------------

    /// An absent cache is "never fetched". An empty catalog would be a claim
    /// that OpenRouter offers nothing.
    #[test]
    fn an_absent_cache_is_never_fetched_rather_than_empty() {
        let slot: CacheSlot<CatalogCache> = CacheSlot::NeverFetched;
        let (badge, age) = slot.badge(None, Some(0));
        assert_eq!(badge, CacheBadge::NeverFetched);
        assert_eq!(age, "never fetched");
        assert!(slot.document().is_none());

        let unreadable: CacheSlot<CatalogCache> = CacheSlot::Unreadable("truncated".to_string());
        assert_eq!(unreadable.badge(None, Some(0)).0, CacheBadge::Unreadable);
    }

    #[test]
    fn the_stale_badge_measures_an_age_rather_than_asserting_one() {
        let now = parse_rfc3339_utc("2026-08-05T12:00:00Z").unwrap();
        let cases = [
            ("2026-08-05T11:59:30Z", CacheBadge::Fresh, "just now"),
            ("2026-08-05T11:00:00Z", CacheBadge::Fresh, "60 min ago"),
            ("2026-08-04T12:00:00Z", CacheBadge::Fresh, "24 h ago"),
            ("2026-08-01T12:00:00Z", CacheBadge::Fresh, "4 days ago"),
            ("2026-07-28T12:00:00Z", CacheBadge::Stale, "8 days ago"),
        ];
        for (stamp, expected, described) in cases {
            let (badge, age) = age_badge(Some(stamp), Some(now));
            assert_eq!(badge, expected, "{stamp}");
            assert_eq!(age, described, "{stamp}");
        }
        // Exactly the boundary is stale, and a future stamp is named rather than
        // printed as a negative age.
        assert_eq!(
            age_badge(Some("2026-07-29T12:00:00Z"), Some(now)).0,
            CacheBadge::Stale
        );
        assert!(age_badge(Some("2026-09-01T00:00:00Z"), Some(now))
            .1
            .contains("clock skew"));
        // An unparsable stamp echoes the file rather than computing from a
        // misreading.
        let (badge, age) = age_badge(Some("last tuesday"), Some(now));
        assert_eq!(badge, CacheBadge::Stale);
        assert!(age.contains("last tuesday"));
    }

    #[test]
    fn the_timestamp_parser_matches_what_the_tools_write() {
        assert_eq!(parse_rfc3339_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_rfc3339_utc("2026-08-04T12:00:00Z"),
            Some(1_785_844_800)
        );
        for bad in [
            "",
            "2026-08-04",
            "2026-08-04 12:00:00Z",
            "2026-08-04T12:00:00+02:00",
            "20260804T120000Z",
            "2026-13-04T12:00:00Z",
            "2026-08-04T25:00:00Z",
        ] {
            assert_eq!(parse_rfc3339_utc(bad), None, "{bad:?} parsed");
        }
    }

    // -- key test ----------------------------------------------------------

    #[test]
    fn every_documented_test_outcome_has_a_distinct_actionable_sentence() {
        let outcomes = [
            "ok",
            "invalid",
            "revoked",
            "rate-limited",
            "no-network",
            "no-key",
        ];
        let mut sentences = std::collections::BTreeSet::new();
        for token in outcomes {
            let outcome = KeyTest::parse(token).expect(token);
            assert_eq!(outcome.token(), token);
            assert!(
                sentences.insert(outcome.sentence()),
                "{token} is not distinct"
            );
        }
        assert_eq!(sentences.len(), outcomes.len());
        assert_eq!(KeyTest::parse("something"), None);
        // The check is the read-only endpoint, never a completion.
        assert_eq!(KEY_TEST_URL, "https://openrouter.ai/api/v1/key");
    }

    // -- sections ----------------------------------------------------------

    #[test]
    fn every_section_token_round_trips() {
        for section in Section::ALL {
            assert_eq!(Section::parse(section.token()), Some(section));
            assert!(!section.title().is_empty());
            assert!(!section.heading().is_empty());
        }
        assert_eq!(Section::parse("1"), Some(Section::Routing));
        assert_eq!(Section::parse("nope"), None);
    }

    /// The report lines a capture carries. Named keys, so a gate greps rather
    /// than parses.
    #[test]
    fn the_report_names_every_state_a_capture_has_to_assert() {
        let mut dialog = dialog();
        dialog.section = Section::OpenRouter;
        dialog.credentials = CredentialState::Refused {
            path: "/tmp/credentials.json".to_string(),
            mode: 0o644,
        };
        let lines = dialog.describe();
        let joined = lines.join("\n");
        for key in [
            "assist settings: section=openrouter",
            "focus=",
            "dirty=false",
            "sections=rail",
            "routing=matrix",
            "assist settings stores:",
            "credentials=refused(0644)",
            "assist settings discovery: catalog=never fetched",
            "codex=never fetched",
            "assist settings routing: TC-MEASURED=",
            "assist settings key: state=refused(0644)",
            "network-allowed=false",
        ] {
            assert!(joined.contains(key), "the report has no {key:?}:\n{joined}");
        }
        assert_eq!(lines.len(), 5);
        let closed = AssistSettingsDialog::new();
        assert_eq!(
            closed.describe(),
            vec!["assist settings: closed".to_string()]
        );
    }

    /// The cycling picker reaches every option and comes back.
    #[test]
    fn cycling_reaches_every_option_and_wraps() {
        let options = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(cycle(&options, None), Some("a".to_string()));
        assert_eq!(
            cycle(&options, Some(&"a".to_string())),
            Some("b".to_string())
        );
        assert_eq!(
            cycle(&options, Some(&"c".to_string())),
            Some("a".to_string())
        );
        // A value that is not in the list starts the cycle rather than stalling.
        assert_eq!(
            cycle(&options, Some(&"z".to_string())),
            Some("a".to_string())
        );
        assert_eq!(cycle::<String>(&[], None), None);
    }

    /// The modal input block, driven through the same claim rule the shell uses.
    ///
    /// One full-window widget drawn into the shell's bank before any panel is
    /// enough, because a press is claimed by the first widget to see it and one
    /// `active_button_id` holds that claim for the whole frame
    /// (`ui_widgets.c:139-160`). The second half of the test is what makes it a
    /// control rather than an assertion about nothing: without the blocker the
    /// very same press does reach the panel.
    #[test]
    fn the_modal_blocker_claims_a_press_no_panel_can_then_take() {
        use crate::ui::widgets::Pointer;

        let scale = UiScale::default();
        let window = UiRect::new(0.0, 0.0, 1280.0, 720.0);
        let control = UiRect::new(100.0, 100.0, 80.0, 30.0);
        let blocker = widgets::widget_id(MODAL_BLOCK_NAMESPACE, 0);
        let panel = widgets::widget_id(7, 3);
        let at = |pressed: bool, released: bool| Pointer {
            x: 120.0,
            y: 110.0,
            down: pressed,
            pressed,
            released,
        };

        let mut blocked = Widgets::new();
        blocked.begin_frame(scale);
        blocked.button_at(blocker, window, at(true, false));
        assert!(!blocked.button_at(panel, control, at(true, false)).clicked);
        blocked.begin_frame(scale);
        blocked.button_at(blocker, window, at(false, true));
        assert!(
            !blocked.button_at(panel, control, at(false, true)).clicked,
            "a press reached a panel underneath the modal"
        );

        let mut open = Widgets::new();
        open.begin_frame(scale);
        open.button_at(panel, control, at(true, false));
        open.begin_frame(scale);
        assert!(
            open.button_at(panel, control, at(false, true)).clicked,
            "the control is unreachable even without the blocker, so the check proves nothing"
        );
    }

    /// Modality eligibility is necessary and not sufficient, and `TC-ALIGN` can
    /// never be served remotely however the catalog is filtered.
    #[test]
    fn modality_eligibility_refuses_the_local_only_contracts_outright() {
        let audio = CatalogModel {
            id: "acme/audio".to_string(),
            input_modalities: vec!["audio".to_string()],
            output_modalities: vec!["text".to_string()],
            ..CatalogModel::default()
        };
        assert!(modalities_fit(ContractId::Semantic, &audio));
        assert!(!modalities_fit(ContractId::Align, &audio));
        assert!(!modalities_fit(ContractId::Measured, &audio));
        let text = CatalogModel {
            id: "acme/text".to_string(),
            input_modalities: vec!["text".to_string()],
            output_modalities: vec!["text".to_string()],
            ..CatalogModel::default()
        };
        assert!(!modalities_fit(ContractId::Coarse, &text));
        assert!(modalities_fit(ContractId::Wording, &text));
    }
}
