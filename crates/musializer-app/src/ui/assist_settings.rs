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

use musializer_core::assist::contracts::{ContractId, FallbackPolicy, RouteType, ALL_CONTRACTS};
use musializer_core::assist::models_dir::ModelsDirResolution;
use musializer_core::assist::secret::Secret;
use musializer_core::assist::settings::{
    AssistSettings, CredentialMode, Profile, Provider, ReasoningEffort, Route, StemSeparation,
    RECOMMENDED_PROFILE, SCHEMA as SETTINGS_SCHEMA,
};
use musializer_core::assist::suitability::{self, Suitability};
use musializer_core::ui::workspace_layout::UiRect;
use musializer_runtime::assist::discover::{self, Discovery};
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

/// Lets the dialog's own tooltips fire, and immediately.
///
/// The dialog draws into a [`Widgets`] bank of its own, so `--ui-probe hover=`'s
/// zeroing of the shell's dwell never reached it — the badge and disabled-control
/// tooltips this tranche added were unphotographable, and a stray pointer left
/// in the middle of an Xvfb screen could pop an unrelated one into a capture on a
/// slow run. Set to `1` to photograph a tip; **any** dialog probe run with this
/// unset holds the dwell at infinity, which is the same rule and the same reason
/// `main.rs` applies to the shell's bank.
pub const PROBE_HOVER_VARIABLE: &str = "MUSIALIZER_ASSIST_SETTINGS_HOVER";

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
    /// The file exists and could not be used, with the three cases separated
    /// (AP3-R S12).
    ///
    /// One `Error(String)` used to carry all of them, and the string it carried
    /// for a schema-invalid *entry* was "the credentials file is not valid JSON
    /// for this schema" — which names no path, no entry and no fix, and is a
    /// factually wrong sentence when the file parses as JSON perfectly well.
    Unusable {
        kind: CredentialFault,
        path: String,
        /// The entry the fault is in, when it is in one. Never its value.
        entry: Option<String>,
    },
}

/// Why a credentials file could not be used. Each is a different fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialFault {
    /// The bytes could not be read at all.
    Io,
    /// The bytes are not JSON.
    NotJson,
    /// The bytes are JSON, but not this schema — a named `schema` mismatch, a
    /// missing `entries` map, or an entry whose shape is wrong.
    Schema,
    /// Over the size cap, refused before parsing.
    TooLarge,
}

impl CredentialFault {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            CredentialFault::Io => "io",
            CredentialFault::NotJson => "not-json",
            CredentialFault::Schema => "schema",
            CredentialFault::TooLarge => "too-large",
        }
    }

    /// What the user does about it. The whole point of separating the cases.
    #[must_use]
    pub const fn remediation(self) -> &'static str {
        match self {
            CredentialFault::Io => {
                "Check that the path exists and that you can read it, then reopen this dialog."
            }
            CredentialFault::NotJson => {
                "The file is not JSON at all. Move it aside and use Replace below to store the key \
                 again; Musializer will not overwrite it while it is still there."
            }
            CredentialFault::Schema => {
                "The file is valid JSON but not a musializer.assist-credentials/v1 store. Each \
                 entry must be an object with a string \"secret\". Repair the named entry, or move \
                 the file aside and use Replace below."
            }
            CredentialFault::TooLarge => {
                "The file is over the size cap and was refused before it was parsed. Move it aside \
                 and use Replace below to store the key again."
            }
        }
    }
}

impl CredentialState {
    #[must_use]
    pub fn token(&self) -> String {
        match self {
            CredentialState::None => "none".to_string(),
            CredentialState::Session { fingerprint } => format!("session({fingerprint})"),
            CredentialState::File { fingerprint, .. } => format!("file({fingerprint})"),
            CredentialState::Refused { mode, .. } => format!("refused({mode:04o})"),
            CredentialState::Unusable { kind, entry, .. } => match entry {
                Some(entry) => format!("unusable({} entry={entry})", kind.token()),
                None => format!("unusable({})", kind.token()),
            },
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
    // There was a `token()` here — `ready` / `blocked` / `unknown` — and it was
    // what the report line carried. It is gone because one word for four
    // different missing pieces is what let a negative control for AP3-R S3 pass:
    // removing the model check swapped `No model chosen` for `No eligible
    // endpoint` and the gate, which could only see `blocked`, noticed nothing.
    // The report carries `label()` instead.

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

    /// The full sentence behind the badge, for its tooltip.
    ///
    /// The badge itself is 96 px wide and its text is ellipsized to fit, which
    /// turned "pip install torchaudio into the alignment venv" into
    /// "pip inst…" — a remediation nobody could act on (AP3-R S9). The label is
    /// the short form and this is the long one, and the badge is hoverable so
    /// that both exist.
    #[must_use]
    pub fn hint(&self) -> String {
        match self {
            Readiness::Ready => {
                "Every piece this route needs is present: its runtime, its model, and a \
                 credential where one is required."
                    .to_string()
            }
            Readiness::Blocked(reason) => match reason.as_str() {
                NO_KEY => format!(
                    "{reason}. OpenRouter needs a key before this route can run. Add one in the \
                     OpenRouter section of this dialog, then Test it."
                ),
                NO_MODEL => format!(
                    "{reason}. The route names a provider but no model, so there is nothing to \
                     send the request to. Pick one in the MODEL column."
                ),
                NO_ENDPOINT => format!(
                    "{reason}. The cached catalog offers no model this contract can use under the \
                     current filters. Refresh the catalog, or turn on Show experimental."
                ),
                CODEX_NOT_FOUND => format!(
                    "{reason}. It is not on this process's PATH, in any common install location, \
                     or on your login shell's PATH. The Codex section lists everywhere this \
                     looked; set local_runtimes.codex_bin to fix it directly."
                ),
                CODEX_OVERRIDE_MISSING => format!(
                    "{reason}. local_runtimes.codex_bin names a path that is not a runnable file, \
                     and an explicit setting is never quietly replaced by a guess. Fix it or \
                     remove it."
                ),
                other => format!("Blocked: {other}"),
            },
            Readiness::Unknown(reason) => {
                format!("{reason}. Nothing has measured this yet, so neither ready nor blocked is a claim this dialog can make.")
            }
        }
    }
}

/// The routing half of this dialog lives in `musializer-core`.
///
/// It was written here for the dry-run summary — "what the **next** job would
/// use" — and tranche P4 needs the identical answer at Start, where it becomes
/// the execution snapshot (`docs/ASSIST_PROVIDER_CONTRACTS.md` §6). Two copies
/// of a routing table is exactly the drift that would put one answer on screen
/// and a different one in a job's provenance, so the resolver moved down to
/// `core::assist::execution` and this is the same function, re-exported. Every
/// call site below is unchanged.
pub use musializer_core::assist::execution::{
    recommended_route, resolve_route, ResolvedRoute, RouteOrigin, CODEX_DEFAULT_LABEL, NO_ENDPOINT,
    NO_KEY, NO_MODEL,
};

/// The user profile the first override creates. `recommended` cannot be stored
/// (§2), so editing while it is active has to copy-on-write into something.
pub const OVERRIDE_PROFILE_ID: &str = "custom";
pub const OVERRIDE_PROFILE_LABEL: &str = "Custom";

/// Which profile [`set_override`] would write to, and whether reaching it means
/// switching the active profile.
///
/// Split out from the write so a test can assert the *decision* without
/// mutating anything, and so the Profile picker can name the target in its
/// tooltip rather than the user discovering it after the fact.
#[must_use]
pub fn override_target(settings: &AssistSettings) -> (String, bool) {
    // The active profile, whatever its id, when it is one the file actually
    // carries. This is the whole of AP3-R S2: the first version force-switched
    // `active_profile` to `custom` on **every** override, so a user with a
    // profile called `studio` had their edit written into a second profile and
    // `studio` left behind, still in the file, never active again. An override
    // is an edit to what is selected, not a reason to select something else.
    if settings.active_profile != RECOMMENDED_PROFILE
        && settings.profile(&settings.active_profile).is_some()
    {
        return (settings.active_profile.clone(), false);
    }
    // The built-in profile is read-only (§2), so an edit made while it is
    // active copies into a user profile. An existing one is reused rather than
    // a second `custom` invented beside it — two near-identical profiles is how
    // a settings file becomes unreadable, and the picker lists them all anyway.
    if let Some(existing) = settings
        .profile(OVERRIDE_PROFILE_ID)
        .or_else(|| settings.profiles.first())
    {
        return (existing.id.clone(), true);
    }
    (OVERRIDE_PROFILE_ID.to_string(), true)
}

/// Every profile the picker offers: `recommended` first, then every profile the
/// file carries, in file order.
///
/// The built-in one is always offered even when the file has none of its own,
/// because it is the thing every absent route inherits from and a picker that
/// could not name it would make the inheritance invisible.
#[must_use]
pub fn profile_options(settings: &AssistSettings) -> Vec<String> {
    let mut options = vec![RECOMMENDED_PROFILE.to_string()];
    options.extend(settings.profiles.iter().map(|profile| profile.id.clone()));
    options
}

/// Stores a per-task override in the **active** profile, copying `recommended`
/// into a user profile first when that is what is active.
///
/// Removing an override that equals the recommended route again drops the entry,
/// so a user who cycles back to the default gets a file that says so rather than
/// one that pins the current default forever.
pub fn set_override(settings: &mut AssistSettings, route: Route) {
    let contract = route.contract;
    let matches_recommended = recommended_route(contract).is_some_and(|base| base == route);
    let (target, switches) = override_target(settings);
    if settings.profile(&target).is_none() {
        if matches_recommended {
            // Nothing to store and nothing to remove: creating a profile to
            // hold the value it already inherits would be a file that pins
            // today's default forever.
            return;
        }
        settings.profiles.push(Profile {
            id: target.clone(),
            label: OVERRIDE_PROFILE_LABEL.to_string(),
            routes: BTreeMap::new(),
        });
    }
    if switches {
        settings.active_profile = target.clone();
    }
    let Some(profile) = settings
        .profiles
        .iter_mut()
        .find(|profile| profile.id == target)
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
/// `assist::suitability`).
///
/// **The currently selected id is not an exception to that** (operator ruling,
/// 2026-08-05). It used to be: the id was re-inserted after the filter so a
/// picker could never drop the value it was showing. The consequence was the
/// defect the ruling settles — `TC-SEMANTIC`'s only model is experimental, so
/// with `Show experimental` off the escape hatch put it straight back and the
/// row displayed `xiaomi/mimo-v2.5` with an orange `experimental` marker while
/// the toggle read `off`. A toggle whose own row contradicts it is worse than
/// no toggle.
///
/// So this list is now strictly *what may be offered*. What is **configured** is
/// a separate question with a separate answer: the MODEL cell still draws the
/// configured id, because hiding it would lie about the configuration, and
/// [`AssistSettingsDialog::routing_model_cell`] says in words that it is not on
/// offer. `selected` is still taken, and still keeps the value in its own list —
/// but only when the value is one the filter would have offered anyway, so
/// cycling stays stable for every model that is legitimately on the list.
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
        // Codex returns before the overlay, on purpose. The overlay is an
        // allowlist of *this repository's* evidence and holds no Codex row, so
        // every discovered Codex id would resolve to `experimental` and the
        // whole section would empty itself the moment the toggle went off. What
        // Codex offers is Codex's own answer to `model/list`, not a Musializer
        // benchmark claim, and §5 rule 6 already governs it.
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
    // A selected id the *catalog* does not carry — a stale cache, a hand-edited
    // profile — is kept, so its own picker can still show and cycle it. A
    // selected id the *overlay* filters out is not, which is the ruling above.
    if let Some(selected) = selected.filter(|id| !id.is_empty()) {
        if !options.iter().any(|id| id == selected)
            && suitability::status(selected, contract).offered(show_experimental)
        {
            options.insert(0, selected.to_string());
        }
    }
    options
}

/// The words drawn under a MODEL cell, describing the **configured** model.
///
/// Defect B, second half. The marker used to be the bare suitability token, so
/// an orange `experimental` sat under a row while `Show experimental: off` sat
/// above it, and nothing on screen said which of the two was lying. Neither was:
/// the model *is* experimental and it *is* not on offer. Both facts are in the
/// text now, and the leading phrase is the one that survives being ellipsized
/// into a 202 px column.
///
/// `None` for a recommended model — a row with nothing to warn about says
/// nothing, which is what makes the warning readable when there is one.
#[must_use]
pub fn model_marker(status: Suitability, show_experimental: bool) -> Option<&'static str> {
    match status {
        Suitability::Recommended => None,
        Suitability::Experimental if show_experimental => Some("configured: experimental"),
        Suitability::Experimental => Some("not offered: experimental"),
        Suitability::Unsupported => Some("not offered: unsupported"),
    }
}

/// The sentence the operator specified for a contract the filter leaves with
/// nothing to offer, verbatim.
pub const NO_OFFERABLE_MODEL: &str = "No recommended model for this task \u{2014} turn on Show \
                                      experimental";

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
/// threshold. 928 is where a rail-mode body reaches the 722 px the six columns
/// need; below it every layout stacks, rail or tabs.
///
/// It was 882 against a 676 px body until AP3-R S9 widened the CONTRACT column
/// to carry a plain-language name. The pair moves together and a test checks
/// that it did.
pub const ROUTING_MATRIX_DIALOG_WIDTH: f32 = 928.0;

/// The narrowest body the six-column matrix fits in: the five fixed columns, the
/// five gaps, and the 90 px floor under the flexible MODEL column. Read by the
/// test that ties the two constants together — the dialog threshold above is
/// only correct *because* it produces at least this body.
#[cfg(test)]
pub const ROUTING_MATRIX_BODY_WIDTH: f32 = 722.0;

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
            // `RAIL_WIDTH - DIALOG_PADDING * 2.0`, not `- DIALOG_PADDING`. The
            // rail's right edge *is* the divider (`content.x` below), so one
            // padding put the buttons flush against the rule while leaving a
            // comfortable gap on the dialog's side — an asymmetry the operator
            // called out by name. Both gaps are now `DIALOG_PADDING`, and
            // `section_gaps` reports them so a capture can prove it.
            for (index, tab) in tabs.iter_mut().enumerate() {
                *tab = UiRect::new(
                    dialog.x + DIALOG_PADDING,
                    body_top + DIALOG_PADDING + index as f32 * 38.0,
                    (RAIL_WIDTH - DIALOG_PADDING * 2.0).max(0.0),
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

    /// The gap on each side of the section list: from the dialog's left border
    /// to the first button, and from the buttons' right edge to whatever bounds
    /// them there — the divider in rail mode, the dialog's right border in tabs
    /// mode.
    ///
    /// Returned as a pair rather than asserted here so the *drawn* geometry is
    /// what the report line and the gate measure. A symmetry nothing photographs
    /// is a symmetry nobody reviews, and this one shipped broken for three
    /// tranches.
    #[must_use]
    pub fn section_gaps(&self) -> (f32, f32) {
        let first = self.tabs[0];
        let last = self.tabs[self.tabs.len() - 1];
        let right_bound = if self.rail_on_left {
            self.content.x
        } else {
            self.dialog.x + self.dialog.width
        };
        let right_edge = if self.rail_on_left {
            first.x + first.width
        } else {
            last.x + last.width
        };
        (first.x - self.dialog.x, right_bound - right_edge)
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

/// The executable discovery looks for.
const CODEX_BINARY: &str = "codex";

/// The two readiness reasons a Codex route can be blocked for.
///
/// Named constants for the reason `NO_KEY` and `NO_MODEL` are: the report line
/// carries the *label*, and a gate that matched on a substring of a sentence
/// would keep passing after the sentence changed meaning. Neither says "not on
/// PATH" any more, because `PATH` is one rung of four.
pub const CODEX_NOT_FOUND: &str = "codex not found";
pub const CODEX_OVERRIDE_MISSING: &str = "codex_bin path is wrong";

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
    /// Where `codex` is, how it was found, and everywhere that was looked.
    ///
    /// Defect C. This was `Option<PathBuf>` from `which("codex")`, and it said
    /// "codex not on PATH" about a binary the operator can run by typing its
    /// name: a GUI process started from a desktop entry inherits the session
    /// manager's minimal `PATH`, not the login shell's. The whole ladder and the
    /// reasons are `musializer_runtime::assist::discover`.
    codex_discovery: Option<Discovery>,
    /// The thorough pass, which may spawn `npm` and a login shell. Polled like
    /// any other background job; it is deliberately **not** in
    /// [`Background`], because it must not disable the buttons that check
    /// `background.is_none()` and it starts without the user pressing anything.
    codex_probe: Option<mpsc::Receiver<Discovery>>,
    /// One thorough pass per process. The cache in `discover` makes a second one
    /// cheap, but a thread per dialog open is still a thread per dialog open.
    codex_probe_done: bool,
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
    /// The traversal ring, refreshed once per frame.
    ///
    /// Cached rather than recomputed on every `is_focused` call: the ring is
    /// built from the live model, and with a 2000-model catalog rebuilding it
    /// per control meant cloning 2000 strings per control per frame. Empty
    /// outside a drawn frame, where [`Self::focused`] computes it instead — so a
    /// unit test that sets `section` and asks never sees a stale answer.
    ring: Vec<ControlId>,
    /// Set by Enter; consumed by the control that owns [`Self::focus`].
    activate: bool,
    /// Escape with unsaved edits asks once rather than discarding (§ the design
    /// document's "no silent discard").
    confirm_close: bool,
    /// A probe Escape held until the frame that consumes a probe Enter has run.
    pending_escape: bool,
    /// The focused control's box, recorded by [`Self::focus_ring`] as it draws.
    ///
    /// This is what makes scroll-follows-focus possible without a second layout
    /// pass: every focusable control already asks for its ring, so the one place
    /// that draws it is the one place that knows where it ended up.
    focused_bounds: Option<UiRect>,
    /// The scrolling viewport from the last drawn frame, which is what
    /// [`Self::focused_bounds`] has to stay inside.
    last_content: Option<UiRect>,
    /// The focus moved and the view has not caught up yet.
    focus_follow: bool,

    scroll: f32,
    /// A scroll offset asked for before anything had been measured.
    ///
    /// [`PROBE_SCROLL_VARIABLE`] is applied at open, where `content_height` is
    /// still zero from the previous section, so clamping it there clamped it to
    /// zero and the seam was dead (AP3-R S10). Held until the first drawn frame
    /// has measured the body instead.
    pending_scroll: Option<f32>,
    /// The body height the last drawn frame used, for PageUp/PageDown.
    last_body_height: f32,
    /// Whether an analysis job is running right now.
    ///
    /// Threaded in from the shell rather than owned here: the dialog reads
    /// files, and "is a helper process alive" is the Assist panel's state
    /// (§5 invariant 3 is what the banner it drives is about).
    job_running: bool,
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
    /// The whole layout from the last drawn frame, so the report can print the
    /// section list's two gaps in pixels rather than the constants they were
    /// derived from. A capture is checked against these numbers.
    last_geometry: Option<DialogLayout>,
    /// The box the "no offerable model" sentence was drawn into last frame.
    ///
    /// Reported so the gate can measure *that* band for ink. Comparing the two
    /// experimental captures cannot do it: turning the toggle on adds its own
    /// warning above the matrix and shifts every row, so every band differs and
    /// a difference proves nothing.
    last_starved_band: Option<UiRect>,
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
            codex_discovery: None,
            codex_probe: None,
            codex_probe_done: false,
            doctor: None,
            doctor_error: None,
            pending_key: None,
            key_test: None,
            key_tested_ok: false,
            test_returns_to_pending: false,
            overwrite_armed: false,
            focus: 0,
            ring: Vec::new(),
            activate: false,
            confirm_close: false,
            pending_escape: false,
            focused_bounds: None,
            last_content: None,
            focus_follow: false,
            scroll: 0.0,
            pending_scroll: None,
            last_body_height: 0.0,
            job_running: false,
            content_height: 0.0,
            background: None,
            status: None,
            now: None,
            last_layout: None,
            last_geometry: None,
            last_starved_band: None,
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

    /// Tells the dialog whether a helper process is alive right now (AP3-R S11).
    ///
    /// One boolean from the shell, refreshed every frame. The dialog states in
    /// its subtitle that changes apply to the next job (§5 invariant 3) and had
    /// no way to know whether there *was* a current one, so the promise was
    /// never shown being kept.
    pub fn set_job_running(&mut self, running: bool) {
        self.job_running = running;
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
        self.ring.clear();
        self.scroll = 0.0;
        self.pending_scroll = None;
        self.focused_bounds = None;
        self.focus_follow = false;
        self.confirm_close = false;
        self.pending_escape = false;
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
                Err(error) => classify_credential_fault(path, &error),
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
        self.resolve_codex();
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

    /// Finds `codex`, cheaply now and thoroughly in a moment.
    ///
    /// Defect C, and the reason it is split in two: the cheap ladder is stat
    /// calls, so it runs inline and the section has an answer on the first
    /// frame; the last two rungs can spawn `npm` and the user's login shell, so
    /// they run on a thread and land whenever they land. Nothing here runs per
    /// frame — `reload` is called on open, and `discover::resolve_cached`
    /// memoizes for the process, so a login shell runs at most once a session.
    fn resolve_codex(&mut self) {
        let override_path = self.draft.local_runtimes.codex_bin.clone();
        let cheap = discover::resolve_cached(
            CODEX_BINARY,
            override_path.as_deref(),
            discover::Options::default(),
        );
        let settled = !matches!(cheap.outcome, discover::Outcome::NotFound);
        self.codex_discovery = Some(cheap);
        if settled || self.codex_probe_done || self.codex_probe.is_some() {
            return;
        }
        if !discover::subprocess_permitted() {
            // The gate sets this. A check that shells out to the operator's real
            // login shell would depend on their rc files, and the headless run
            // exists to be independent of the session it runs in.
            self.codex_probe_done = true;
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(discover::resolve_cached(
                CODEX_BINARY,
                override_path.as_deref(),
                discover::Options::thorough(),
            ));
        });
        self.codex_probe = Some(receiver);
    }

    /// The resolved `codex`, when there is one.
    fn codex_binary(&self) -> Option<&Path> {
        self.codex_discovery.as_ref().and_then(Discovery::path)
    }

    /// Collects the thorough pass. Separate from [`Self::poll_background`]
    /// because it holds no user-facing slot: it starts on its own and must never
    /// make Save or Refresh look busy.
    fn poll_codex_probe(&mut self) {
        let Some(receiver) = &self.codex_probe else {
            return;
        };
        match receiver.try_recv() {
            Ok(discovery) => {
                self.codex_probe = None;
                self.codex_probe_done = true;
                // The thorough pass only ever *improves* on the cheap one, so a
                // `NotFound` from it replaces a `NotFound` and nothing else.
                self.codex_discovery = Some(discovery);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.codex_probe = None;
                self.codex_probe_done = true;
            }
        }
    }

    /// The probe seams, applied after a real load so a capture photographs the
    /// real surface with one fact changed rather than a mock of it.
    fn apply_probe_state(&mut self) {
        // A probe-driven run is one where `_OPEN` put the dialog here. In one,
        // the dwell is infinite unless `_HOVER` asks for a tip, so a capture
        // never depends on where the X server happened to leave the pointer.
        if std::env::var_os(PROBE_OPEN_VARIABLE).is_some() {
            self.widgets.tooltip_delay =
                if std::env::var(PROBE_HOVER_VARIABLE).is_ok_and(|text| text.trim() == "1") {
                    0.0
                } else {
                    f64::INFINITY
                };
        }
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
                // Deferred to the first drawn frame. Setting `scroll` here put
                // the offset in front of a clamp against a content height
                // nothing had measured yet, so every value clamped to zero and
                // no section bottom was ever photographed (AP3-R S10).
                self.pending_scroll = Some(offset.max(0.0));
            }
        }
        if let Ok(text) = std::env::var(PROBE_ACTIVATE_VARIABLE) {
            if text.trim() == "1" {
                self.activate = true;
            }
        }
        if let Ok(text) = std::env::var(PROBE_ESCAPE_VARIABLE) {
            if text.trim() == "1" {
                if self.activate {
                    // Enter and then Escape: the Enter is only consumed by a
                    // control while one is being *drawn*, so closing here would
                    // throw it away and the pair would silently test the Escape
                    // alone. Deferred by exactly one frame, which is the shortest
                    // gap a headless run can put between two key presses. Alone,
                    // Escape stays immediate — that keeps "the dialog prints
                    // nothing at all once closed" assertable.
                    self.pending_escape = true;
                } else {
                    self.escape();
                }
            }
        }
    }

    // -- one expression per control ----------------------------------------
    //
    // AP3-R S5. Every one of these is called from exactly two places: the ring
    // builder below, and the control's own drawing code. They exist because the
    // two were written separately the first time and drifted — the ring offered
    // ~15 tabstops on a fresh install that landed on controls drawn disabled,
    // and Tab walked through every one of them doing nothing. A predicate that
    // cannot be stated twice cannot disagree with itself.

    /// Whether the settings on screen may be edited at all.
    ///
    /// False when the file failed to load: the draft started from the built-in
    /// defaults rather than from the user's file, so an edit would be an edit to
    /// something they never chose, and Save over it is already an explicit
    /// two-press action (AP3-R S1).
    #[must_use]
    fn settings_editable(&self) -> bool {
        !matches!(self.settings_source, SettingsSource::Error(_))
    }

    fn save_enabled(&self) -> bool {
        self.is_dirty() && !matches!(self.settings_source, SettingsSource::NoPath)
    }

    fn profile_picker_enabled(&self) -> bool {
        self.settings_editable() && profile_options(&self.draft).len() > 1
    }

    fn experimental_toggle_enabled(&self) -> bool {
        self.settings_editable()
    }

    fn route_cell_enabled(&self, contract: ContractId) -> bool {
        self.settings_editable()
            && !contract.is_locked()
            && contract.eligible_route_types().len() > 1
    }

    /// Enabled when the picker can reach a value it is not already showing.
    ///
    /// `len() > 1` until defect B, which was the same test only while the
    /// selected id was force-fed back into the list. It is not any more, so the
    /// interesting case is a list of exactly one option that is *not* what is
    /// configured — the one press that moves `TC-SEMANTIC` off an experimental
    /// model and onto the offered one. `len() > 1` would have drawn that
    /// disabled.
    fn model_cell_enabled(&self, contract: ContractId) -> bool {
        if !self.settings_editable() || contract.is_locked() {
            return false;
        }
        let configured = self.configured_model(contract);
        self.model_options_for(contract)
            .iter()
            .any(|option| Some(option.as_str()) != configured.as_deref())
    }

    /// The model id the settings name for a contract, whether or not any picker
    /// would offer it. The configuration is a fact about the file; what is
    /// offered is a fact about the filter, and defect B is what happens when one
    /// surface tries to be both.
    fn configured_model(&self, contract: ContractId) -> Option<String> {
        resolve_route(&self.draft, contract)
            .route
            .and_then(|route| route.model_id)
            .filter(|id| !id.is_empty())
    }

    /// The overlay's verdict on the configured model, when there is one.
    fn configured_status(&self, contract: ContractId) -> Option<Suitability> {
        self.configured_model(contract)
            .map(|id| suitability::status(&id, contract))
    }

    /// The contracts whose MODEL picker has nothing at all to offer, and whose
    /// list `Show experimental` would refill.
    ///
    /// Excludes the locked contract and any contract with no route: neither has
    /// a picker for the toggle to be about, and naming them would send the user
    /// to a control that would not change anything.
    fn contracts_without_an_offer(&self) -> Vec<ContractId> {
        if self.draft.catalog.show_experimental {
            return Vec::new();
        }
        ALL_CONTRACTS
            .into_iter()
            .filter(|contract| !contract.is_locked())
            .filter(|contract| resolve_route(&self.draft, *contract).route.is_some())
            .filter(|contract| self.model_options_for(*contract).is_empty())
            .filter(|contract| !self.model_options_with_experimental(*contract).is_empty())
            .collect()
    }

    /// AP3-R S6: `TC-VERIFY` has no recommended route, so its fallback picker
    /// had nothing to write a policy onto and silently did nothing when pressed.
    fn fallback_cell_enabled(&self, contract: ContractId) -> bool {
        self.settings_editable()
            && !contract.is_locked()
            && contract.allowed_fallbacks().len() > 1
            && resolve_route(&self.draft, contract).route.is_some()
    }

    fn doctor_enabled(&self) -> bool {
        self.background.is_none()
    }

    fn codex_effort_enabled(&self, contract: ContractId) -> bool {
        self.settings_editable()
            && resolve_route(&self.draft, contract)
                .route
                .is_some_and(|route| route.route_type == RouteType::Codex)
    }

    fn codex_refresh_enabled(&self) -> bool {
        self.background.is_none() && self.codex_binary().is_some()
    }

    fn key_test_enabled(&self) -> bool {
        self.background.is_none() && (self.pending_key.is_some() || self.credentials.is_usable())
    }

    fn key_replace_enabled(&self) -> bool {
        self.background.is_none() && self.pending_key.is_some() && self.key_tested_ok
    }

    fn key_untested_enabled(&self) -> bool {
        self.background.is_none() && self.pending_key.is_some()
    }

    fn key_forget_enabled(&self) -> bool {
        self.background.is_none() && matches!(self.credentials, CredentialState::File { .. })
    }

    fn catalog_refresh_enabled(&self) -> bool {
        self.background.is_none() && self.draft.catalog.network_allowed
    }

    fn zdr_enabled(&self, contract: ContractId) -> bool {
        self.settings_editable()
            && resolve_route(&self.draft, contract)
                .route
                .is_some_and(|route| route.route_type == RouteType::OpenRouter)
    }

    /// Which controls Tab walks, in visual order, for the open section.
    ///
    /// Built from the live model rather than from a constant list, and filtered
    /// by the very predicates the drawing code uses: a control drawn disabled is
    /// not a tabstop, because a ring that stops somewhere Enter does nothing is
    /// a ring that has taught the user Enter does nothing.
    #[must_use]
    pub fn focus_order(&self) -> Vec<ControlId> {
        let mut order = Vec::new();
        if self.save_enabled() {
            order.push(FOCUS_SAVE);
        }
        order.push(FOCUS_CLOSE);
        for (index, _) in Section::ALL.iter().enumerate() {
            order.push(FOCUS_TAB_BASE + index as ControlId);
        }
        match self.section {
            Section::Routing => {
                if self.profile_picker_enabled() {
                    order.push(FOCUS_ROUTING_PROFILE);
                }
                if self.experimental_toggle_enabled() {
                    order.push(FOCUS_ROUTING_EXPERIMENTAL);
                }
                for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
                    let index = index as ControlId;
                    if self.route_cell_enabled(contract) {
                        order.push(FOCUS_ROUTE_BASE + index);
                    }
                    if self.model_cell_enabled(contract) {
                        order.push(FOCUS_MODEL_BASE + index);
                    }
                    if self.fallback_cell_enabled(contract) {
                        order.push(FOCUS_FALLBACK_BASE + index);
                    }
                }
            }
            Section::LocalModels => {
                order.push(FOCUS_MODELS_DIR);
                order.push(FOCUS_MODELS_GPU);
                order.push(FOCUS_MODELS_STEMS);
                if self.doctor_enabled() {
                    order.push(FOCUS_MODELS_DOCTOR);
                }
            }
            Section::Codex => {
                for (index, contract) in CODEX_ELIGIBLE.into_iter().enumerate() {
                    if self.codex_effort_enabled(contract) {
                        order.push(FOCUS_CODEX_EFFORT_BASE + index as ControlId);
                    }
                }
                if self.codex_refresh_enabled() {
                    order.push(FOCUS_CODEX_REFRESH);
                }
            }
            Section::OpenRouter => {
                order.push(FOCUS_KEY_FIELD);
                if self.key_test_enabled() {
                    order.push(FOCUS_KEY_TEST);
                }
                if self.key_replace_enabled() {
                    order.push(FOCUS_KEY_REPLACE);
                }
                if self.key_untested_enabled() {
                    order.push(FOCUS_KEY_SAVE_UNTESTED);
                }
                if self.key_forget_enabled() {
                    order.push(FOCUS_KEY_FORGET);
                }
                order.push(FOCUS_CATALOG_NETWORK);
                order.push(FOCUS_CATALOG_REFRESH_ON_OPEN);
                if self.catalog_refresh_enabled() {
                    order.push(FOCUS_CATALOG_REFRESH);
                }
            }
            Section::Privacy => {
                for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
                    if contract.max_boundary().rank() >= 2 && self.zdr_enabled(contract) {
                        order.push(FOCUS_PRIVACY_ZDR_BASE + index as ControlId);
                    }
                }
                order.push(FOCUS_PRIVACY_COPY);
            }
        }
        order
    }

    /// Rebuilds [`Self::ring`]. Called once per drawn frame and whenever the
    /// focus moves, never per control.
    fn refresh_ring(&mut self) {
        self.ring = self.focus_order();
    }

    /// The focused control, or `None` when the ring is empty.
    #[must_use]
    pub fn focused(&self) -> Option<ControlId> {
        if self.ring.is_empty() {
            let order = self.focus_order();
            return order.get(self.focus % order.len().max(1)).copied();
        }
        self.ring.get(self.focus % self.ring.len()).copied()
    }

    /// Where the ring is, for the report line: `index/total`.
    #[must_use]
    pub fn focus_position(&self) -> (usize, usize) {
        let total = if self.ring.is_empty() {
            self.focus_order().len()
        } else {
            self.ring.len()
        };
        (if total == 0 { 0 } else { self.focus % total }, total)
    }

    fn focus_next(&mut self, step: isize) {
        // Traversing is going back to editing, so it cancels a pending "Escape
        // again to discard". Here rather than in the keyboard handler, so a
        // press that moves the focus cancels it too.
        self.confirm_close = false;
        self.refresh_ring();
        let total = self.ring.len();
        if total == 0 {
            self.focus = 0;
            return;
        }
        let current = (self.focus % total) as isize;
        self.focus = (current + step).rem_euclid(total as isize) as usize;
        // AP3-R S4: the view follows the focus rather than the focus being
        // allowed to leave the view. A parallel "scrolling feature" would have
        // left off-screen focus stateable; this makes it unstateable, which is
        // why it is here and not in the keyboard handler.
        self.focus_follow = true;
    }

    /// Whether `id` has the focus ring this frame.
    fn is_focused(&self, id: ControlId) -> bool {
        self.focused() == Some(id)
    }

    /// How far the body can scroll, from the last frame's measurement.
    ///
    /// Reported alongside `scroll=` so a capture can say "this **is** the bottom"
    /// rather than only "this is 261 px down". A section that happens to fit
    /// entirely is at its bottom with a scroll of zero, and a check that only
    /// looked at the offset could not tell that from a dead seam (AP3-R S10).
    fn scroll_overflow(&self) -> f32 {
        (self.content_height - self.last_body_height).max(0.0)
    }

    /// Whether the focused control is one the body scrolls.
    ///
    /// Save, Close and the five section tabs live in the chrome, which does not
    /// move — following the focus onto one of them would scroll the body for a
    /// control that is already visible and never in it.
    fn focus_is_in_body(&self) -> bool {
        match self.focused() {
            Some(FOCUS_SAVE | FOCUS_CLOSE) | None => false,
            Some(id) => {
                !(FOCUS_TAB_BASE..FOCUS_TAB_BASE + Section::ALL.len() as ControlId).contains(&id)
            }
        }
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
        self.model_options_at(contract, self.draft.catalog.show_experimental)
    }

    /// What the same picker would offer with `Show experimental` on.
    ///
    /// Only ever compared with [`Self::model_options_for`], to answer whether
    /// the experimental filter is what left a picker with one option (AP3-R S8).
    fn model_options_with_experimental(&self, contract: ContractId) -> Vec<String> {
        self.model_options_at(contract, true)
    }

    fn model_options_at(&self, contract: ContractId, show_experimental: bool) -> Vec<String> {
        let resolved = resolve_route(&self.draft, contract);
        let Some(route) = &resolved.route else {
            return Vec::new();
        };
        model_options(
            contract,
            route.route_type,
            self.catalog.document(),
            self.codex.document(),
            show_experimental,
            route.model_id.as_deref(),
        )
    }

    /// How many catalog entries this contract could actually be pointed at,
    /// ignoring whatever is currently selected.
    ///
    /// Deliberately **not** [`Self::model_options_for`]: that one always
    /// includes the selected id, because a picker that dropped the value it is
    /// showing could not be used to change it. Readiness is the opposite
    /// question — whether the constraints leave anything at all — so the escape
    /// hatch has to be off, or a route pointed at a model the catalog has never
    /// heard of would count as its own evidence that it can run.
    fn openrouter_eligible(&self, contract: ContractId) -> usize {
        model_options(
            contract,
            RouteType::OpenRouter,
            self.catalog.document(),
            self.codex.document(),
            self.draft.catalog.show_experimental,
            None,
        )
        .len()
    }

    /// Readiness for one resolved route.
    ///
    /// §5 invariant 4: **every** required piece has to be present, and the badge
    /// names the *first* one that is not. Before AP3-R S3 this answered `Ready`
    /// for an OpenRouter route the moment a credential existed — with no model
    /// chosen and no eligible catalog entry — which is the one direction a
    /// readiness badge must never be wrong in. The order is the order the
    /// pieces are needed in: a credential, then something to send, then somewhere
    /// that would accept it.
    ///
    /// Every "not ready" answer names what to do about it, and every "we have not
    /// looked" answer says so rather than claiming a failure.
    #[must_use]
    pub fn readiness(&self, resolved: &ResolvedRoute) -> Readiness {
        let Some(route) = &resolved.route else {
            return Readiness::Blocked("No route chosen".to_string());
        };
        // 1. The credential, where the route type needs one. Codex authenticates
        //    itself and the local lanes open no socket, so only OpenRouter does.
        if route.route_type == RouteType::OpenRouter && !self.credentials.is_usable() {
            return Readiness::Blocked(NO_KEY.to_string());
        }
        // 2. Something to send. A Codex route with no `model_id` is the
        //    documented `Codex default` (§5 rule 6), which is a choice; every
        //    other route type with no model has nothing to run.
        let has_model = match (route.route_type, &route.model_id) {
            (RouteType::Builtin | RouteType::Codex, _) => true,
            (_, Some(id)) => !id.is_empty(),
            (_, None) => false,
        };
        if !has_model {
            return Readiness::Blocked(NO_MODEL.to_string());
        }
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
                    Some(identity) => Readiness::Blocked(sanitize_display(
                        identity
                            .remediation
                            .as_deref()
                            .unwrap_or(identity.state.as_str()),
                    )),
                }
            }
            // Defect C. "not on PATH" was a claim the dialog could not support:
            // `PATH` is one rung of four, and it is the rung a desktop-entry
            // launch is worst at. Each answer now names what actually happened,
            // and the one that means "we have not finished looking" is `Unknown`
            // rather than a confident refusal.
            RouteType::Codex => match self.codex_discovery.as_ref().map(|d| &d.outcome) {
                Some(discover::Outcome::Found { .. }) => Readiness::Ready,
                Some(discover::Outcome::OverrideMissing { .. }) => {
                    Readiness::Blocked(CODEX_OVERRIDE_MISSING.to_string())
                }
                Some(discover::Outcome::NotFound) if self.codex_probe.is_some() => {
                    Readiness::Unknown("Looking for codex".to_string())
                }
                Some(discover::Outcome::NotFound) => {
                    Readiness::Blocked(CODEX_NOT_FOUND.to_string())
                }
                None => Readiness::Unknown("Looking for codex".to_string()),
            },
            // 3. Somewhere that would accept it. An absent catalog is "we have
            //    not looked", not "there is nothing" — the same distinction the
            //    never-fetched badge makes, and the reason this is `Unknown`
            //    rather than a blocked route with a confident-sounding reason.
            RouteType::OpenRouter => match self.catalog.document() {
                None => Readiness::Unknown("Catalog never fetched".to_string()),
                Some(_) if self.openrouter_eligible(resolved.contract) == 0 => {
                    Readiness::Blocked(NO_ENDPOINT.to_string())
                }
                Some(_) => Readiness::Ready,
            },
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
        // The focused control's box and whether it is inside the scrolling
        // viewport. Reported rather than inferred, because "Tab walked somewhere
        // below the fold" and "Tab walked somewhere visible" are the same
        // picture otherwise (AP3-R S4).
        let (focus_rect, focus_visible) = match (self.focused_bounds, self.last_content) {
            (Some(rect), Some(content)) => (
                format!(
                    "{:.0},{:.0},{:.0},{:.0}",
                    rect.x, rect.y, rect.width, rect.height
                ),
                (rect.y >= content.y - 1.0
                    && rect.y + rect.height <= content.y + content.height + 1.0)
                    || !self.focus_is_in_body(),
            ),
            _ => ("none".to_string(), true),
        };
        let mut lines = vec![format!(
            "assist settings: section={} focus={focus}/{total} control={focused} dirty={} confirm-close={} scroll={:.0} overflow={:.0} at-bottom={} sections={} routing={} busy={} job-running={} load-error={} focus-rect={focus_rect} focus-visible={focus_visible}",
            self.section.token(),
            self.is_dirty(),
            self.confirm_close,
            self.scroll,
            self.scroll_overflow(),
            self.scroll >= self.scroll_overflow() - 0.5,
            if rail { "rail" } else { "tabs" },
            if stacked { "stacked" } else { "matrix" },
            self.background
                .as_ref()
                .map_or("none", Background::label),
            self.job_running,
            !self.settings_editable(),
        )];
        // The section list's own geometry, in the pixels a capture can be
        // measured in. The gap to the divider and the gap to the dialog's border
        // must be equal; the operator caught them at 0 and 14 (defect A), and a
        // number in the report is what lets the gate catch a regression without
        // anybody looking at a picture.
        if let Some(layout) = self.last_geometry {
            let (left, right) = layout.section_gaps();
            let first = layout.tabs[0];
            let last = layout.tabs[layout.tabs.len() - 1];
            lines.push(format!(
                "assist settings chrome: mode={} dialog-x={:.0} dialog-w={:.0} first-tab={:.0},{:.0},{:.0},{:.0} last-tab-right={:.0} bound-x={:.0} gap-left={:.0} gap-right={:.0} symmetric={}",
                if layout.rail_on_left { "rail" } else { "tabs" },
                layout.dialog.x,
                layout.dialog.width,
                first.x,
                first.y,
                first.width,
                first.height,
                last.x + last.width,
                if layout.rail_on_left {
                    layout.content.x
                } else {
                    layout.dialog.x + layout.dialog.width
                },
                left,
                right,
                (left - right).abs() <= 0.5,
            ));
        }
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
            self.codex_discovery
                .as_ref()
                .and_then(Discovery::path)
                .map_or_else(|| "absent".to_string(), |path| path.display().to_string()),
            match (&self.doctor, &self.doctor_error) {
                (Some(_), _) => "loaded".to_string(),
                (None, Some(error)) => format!("error({error})"),
                (None, None) => "not run".to_string(),
            },
        ));
        // Defect C's evidence. `codex-bin=absent` above cannot distinguish "it
        // is not installed" from "this process's PATH is the desktop entry's",
        // which is the whole defect — so the method, the override state and the
        // list of places looked in are their own line. The searched list is what
        // a user acts on, so it is printed rather than counted.
        if let Some(discovery) = &self.codex_discovery {
            lines.push(format!(
                "assist settings codex: {} probing={} subprocess={} shell={} override={} searched={}",
                discovery.summary(),
                self.codex_probe.is_some(),
                discovery.consulted_subprocess,
                discover::shell_label(),
                self.draft
                    .local_runtimes
                    .codex_bin
                    .as_deref()
                    .unwrap_or("<unset>"),
                discovery.searched.len(),
            ));
            if discovery.path().is_none() {
                lines.push(format!(
                    "assist settings codex searched: {}",
                    discovery.searched.join(" | ")
                ));
            }
        }
        let routes: Vec<String> = ALL_CONTRACTS
            .into_iter()
            .map(|contract| {
                let resolved = resolve_route(&self.draft, contract);
                // The readiness *label*, not its token. `blocked` is one word for
                // four different missing pieces, and a gate that could only see
                // the word could not tell "no key" from "no model chosen" —
                // which is exactly how a negative control for AP3-R S3 passed
                // while the check it perturbed was disabled.
                format!(
                    "{}={}/{}[{}]{}",
                    contract.token(),
                    resolved.compact(),
                    resolved.model_label(),
                    self.readiness(&resolved).label(),
                    if resolved.origin == RouteOrigin::Override {
                        "*"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        lines.push(format!("assist settings routing: {}", routes.join(" ")));
        // Defect B's evidence. The *offer* and the *configuration* are printed
        // as separate fields per contract, because the defect was the two being
        // read off one string — a capture showing `xiaomi/mimo-v2.5` under
        // `Show experimental: off` is correct or wrong depending on which of the
        // two the row is claiming to be, and only this line says.
        let pickers: Vec<String> = ALL_CONTRACTS
            .into_iter()
            .filter(|contract| !contract.is_locked())
            .map(|contract| {
                let configured = self.configured_model(contract);
                let options = self.model_options_for(contract);
                let status = self.configured_status(contract);
                format!(
                    "{}{{configured={} status={} marker=\"{}\" offers=[{}]}}",
                    contract.token(),
                    configured.as_deref().unwrap_or("<none>"),
                    status.map_or("<none>", Suitability::token),
                    status
                        .and_then(|status| model_marker(
                            status,
                            self.draft.catalog.show_experimental
                        ))
                        .unwrap_or(""),
                    options.join(","),
                )
            })
            .collect();
        lines.push(format!(
            "assist settings pickers: show-experimental={} starved=[{}] starved-band={} {}",
            self.draft.catalog.show_experimental,
            self.contracts_without_an_offer()
                .iter()
                .map(|contract| contract.token().to_string())
                .collect::<Vec<_>>()
                .join(","),
            self.last_starved_band.map_or_else(
                || "none".to_string(),
                |band| format!(
                    "{:.0},{:.0},{:.0},{:.0}",
                    band.x, band.y, band.width, band.height
                )
            ),
            pickers.join(" "),
        ));
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

/// Separates "could not read it", "it is not JSON" and "it is JSON but an entry
/// is wrong", and names the offending entry (AP3-R S12).
///
/// The core store returns one `Format` error for the last two, because it stops
/// at the first serde failure and serde does not distinguish them. So the second
/// pass happens here — and it is deliberately **structural**: the file is parsed
/// as a bare `serde_json::Value` and only *key names* and the JSON *type* of the
/// `secret` member are ever looked at. No `CredentialEntry` is deserialized and
/// no value is read, so no secret is materialized by the diagnosis, let alone
/// displayed. The canary check in the gate covers the claim.
fn classify_credential_fault(path: &Path, error: &AssistFileError) -> CredentialState {
    use musializer_core::assist::credentials::CredentialsError;
    let display = path.display().to_string();
    let unusable = |kind, entry| CredentialState::Unusable {
        kind,
        path: display.clone(),
        entry,
    };
    match error {
        AssistFileError::Read(_) | AssistFileError::Directory(_) | AssistFileError::Write(_) => {
            unusable(CredentialFault::Io, None)
        }
        AssistFileError::Credentials(CredentialsError::TooLarge) => {
            unusable(CredentialFault::TooLarge, None)
        }
        AssistFileError::Credentials(CredentialsError::Schema(_)) => {
            unusable(CredentialFault::Schema, None)
        }
        AssistFileError::Credentials(CredentialsError::Invalid(_)) => {
            unusable(CredentialFault::Schema, first_malformed_entry(path))
        }
        AssistFileError::Credentials(CredentialsError::Format) => {
            let Ok(bytes) = std::fs::read(path) else {
                return unusable(CredentialFault::Io, None);
            };
            match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Err(_) => unusable(CredentialFault::NotJson, None),
                Ok(_) => unusable(CredentialFault::Schema, first_malformed_entry(path)),
            }
        }
        _ => unusable(CredentialFault::Io, None),
    }
}

/// The name of the first entry whose shape is wrong, if the file is JSON.
///
/// Names only. The `secret` member is inspected with `is_string()` and never
/// read, which is the whole reason this walks a `Value` instead of asking the
/// typed parser to tell it more.
fn first_malformed_entry(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let document: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let entries = document.get("entries")?.as_object()?;
    for (name, entry) in entries {
        let well_formed = entry.as_object().is_some_and(|entry| {
            entry
                .get("secret")
                .is_some_and(serde_json::Value::is_string)
        });
        if !well_formed {
            return Some(sanitize_display(name));
        }
    }
    None
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

// There was a `which` here. It is gone, and its removal is defect C: `PATH`
// membership was the *only* thing the dialog asked, so a GUI process started
// from a desktop entry — which inherits the session manager's minimal PATH —
// reported "codex not on PATH" about a binary the operator runs by name every
// day. The replacement is `musializer_runtime::assist::discover`, which asks
// four questions in order and can say which one answered.

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
const DIALOG_WIDGETS: u32 = widgets::id::ASSIST_DIALOG;

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
        self.poll_codex_probe();
        self.keyboard(d);
        if !self.open {
            return;
        }
        self.widgets.begin_frame(ui_scale);
        // Once per frame, before anything asks whether it has the focus. See
        // the field comment: this used to be rebuilt inside every `is_focused`.
        self.refresh_ring();
        self.focused_bounds = None;
        let layout = DialogLayout::of(window);
        self.last_layout = Some((layout.rail_on_left, layout.routing_compact()));
        self.last_content = Some(layout.content);
        self.last_geometry = Some(layout);
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
        if self.pending_escape {
            self.pending_escape = false;
            self.escape();
        }
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
        self.last_body_height = body.height;
        // The scroll bound uses the height the *previous* frame measured; see
        // the field comment. It is clamped before drawing, so a section that got
        // shorter cannot leave the view past its own end for a frame.
        let overflow = (self.content_height - body.height).max(0.0);
        // A scroll asked for before the body had ever been measured is applied
        // here, on the first frame that has a measurement to clamp it against
        // (AP3-R S10).
        if self.content_height > 0.0 {
            if let Some(pending) = self.pending_scroll.take() {
                self.scroll = pending;
            }
        }
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

        // AP3-R S4: the view follows the focus. Applied after the body has drawn
        // because that is when the focused control's box is known — an
        // immediate-mode body has no layout to consult beforehand — so the
        // correction lands on the next frame, which is one frame and invisible.
        if self.focus_follow {
            if let (Some(rect), true) = (self.focused_bounds, self.focus_is_in_body()) {
                let overflow = (self.content_height - body.height).max(0.0);
                let top_edge = layout.content.y + 8.0;
                let bottom_edge = layout.content.y + layout.content.height - 8.0;
                let delta = if rect.y < top_edge {
                    rect.y - top_edge
                } else if rect.y + rect.height > bottom_edge {
                    rect.y + rect.height - bottom_edge
                } else {
                    0.0
                };
                if delta != 0.0 {
                    self.scroll = (self.scroll + delta).clamp(0.0, overflow);
                }
                self.focus_follow = false;
            } else if self.focused_bounds.is_some() {
                // Chrome control: nothing to follow, and leaving the flag armed
                // would make the next body draw jump for it.
                self.focus_follow = false;
            }
        }

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

    /// Draws the focus ring, and records where it went.
    ///
    /// The recording is what makes scroll-follows-focus a property rather than a
    /// second feature: every focusable control in this file already calls this,
    /// including the disabled ones, so there is no control whose position the
    /// scroller can fail to know.
    fn focus_ring(&mut self, d: &mut RaylibDrawHandle<'_>, boundary: UiRect, id: ControlId) {
        if !self.is_focused(id) || boundary.is_empty() {
            return;
        }
        self.focused_bounds = Some(boundary);
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
        let id = widgets::widget_id(DIALOG_WIDGETS, widget);
        if !enabled {
            // AP3-R S5: the hint comes *before* the early return. A disabled
            // control with no tooltip is the one control on screen that cannot
            // say why it is disabled, which is exactly the case a tooltip is
            // for. The hover state comes from a real widget registration —
            // `disabled_button` draws and nothing else — and the claim it takes
            // is wanted anyway: a press on a disabled control inside a modal
            // should land nowhere rather than on whatever is underneath.
            let state = self.widgets.button(d, id, boundary);
            self.widgets
                .disabled_button(d, font, boundary, label, Some(metric::UI_FONT_CAPTION));
            self.widgets.hint(d, state, id, boundary, tip);
            self.focus_ring(d, boundary, focus);
            let _ = self.take_activation(focus);
            return false;
        }
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
        enabled: bool,
    ) -> bool {
        if boundary.is_empty() {
            return false;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, widget);
        if !enabled {
            let state = self.widgets.button(d, id, boundary);
            self.widgets
                .disabled_button(d, font, boundary, label, Some(metric::UI_FONT_CAPTION));
            self.widgets.hint(d, state, id, boundary, tip);
            self.focus_ring(d, boundary, focus);
            let _ = self.take_activation(focus);
            return false;
        }
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

    /// A readiness pill with a tooltip carrying the whole remediation.
    ///
    /// The badge is 96 px and ellipsizes; the sentence a user has to act on is
    /// longer than that in every interesting case (AP3-R S9). Registering the
    /// box as a widget is what makes it hoverable at all — `badge` is a free
    /// function that only draws.
    fn readiness_badge(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        widget: u32,
        boundary: UiRect,
        readiness: &Readiness,
    ) {
        if boundary.is_empty() {
            return;
        }
        let id = widgets::widget_id(DIALOG_WIDGETS, widget);
        let state = self.widgets.button(d, id, boundary);
        badge(d, font, boundary, &readiness.label(), readiness.tint());
        self.widgets.hint(d, state, id, boundary, &readiness.hint());
    }

    /// The banner drawn at the top of **every** section body.
    ///
    /// Two facts, both of which used to be invisible where the user was looking:
    ///
    /// - **A failed settings load** (AP3-R S1). It was reported only on Privacy,
    ///   and the reviewer measured a routing capture of a corrupt file as
    ///   pixel-identical to a clean one — the matrix drew the built-in defaults
    ///   and said nothing. A banner on every section, plus the matrix drawn
    ///   explicitly disabled, is what makes the two states distinguishable
    ///   wherever you are standing.
    /// - **A job in flight** (AP3-R S11). The dialog promises that changes apply
    ///   to the next job; that promise is only meaningful when it can say
    ///   whether there is a current one.
    fn draw_banners(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        body: UiRect,
        cursor: &mut f32,
    ) {
        if self.job_running {
            banner(
                d,
                font,
                body,
                cursor,
                "A job is running \u{2014} these settings apply to the next job. The running one \
                 keeps the route graph it snapshotted at Start.",
                color::ui_warning(),
            );
        }
        match &self.settings_source {
            SettingsSource::Error(reason) => {
                let path = self.settings_path.as_ref().map_or_else(
                    || "<no path>".to_string(),
                    |path| path.display().to_string(),
                );
                let text = format!(
                    "Settings could not be loaded: {reason}. {path} is left exactly as it is, and \
                     everything below shows the built-in defaults rather than your file \u{2014} so \
                     the controls are disabled. Repair or move the file, or see Privacy \u{2192} \
                     Configuration provenance.",
                );
                banner(d, font, body, cursor, &text, color::ui_danger());
            }
            SettingsSource::NoPath => {
                banner(
                    d,
                    font,
                    body,
                    cursor,
                    "No per-user configuration directory could be resolved, so nothing here can be \
                     saved. Set XDG_CONFIG_HOME or HOME. See Privacy \u{2192} Configuration \
                     provenance.",
                    color::ui_danger(),
                );
            }
            SettingsSource::Absent | SettingsSource::Loaded => {}
        }
    }
}

/// A bordered notice block, drawn full-width at the top of a section body.
fn banner(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    text: &str,
    tint: Color,
) {
    if body.width <= 24.0 {
        return;
    }
    let lines = wrap(font, text, body.width - 24.0, metric::UI_FONT_CAPTION);
    let height = lines.len() as f32 * 17.0 + 12.0;
    let boundary = UiRect::new(body.x, *cursor, body.width, height);
    widgets::fill(d, boundary, color::ui_raised());
    d.draw_rectangle_lines_ex(widgets::rectangle(boundary), 1.0, tint);
    widgets::fill(d, UiRect::new(boundary.x, boundary.y, 3.0, height), tint);
    let mut y = *cursor + 6.0;
    for line in lines {
        widgets::draw_text(
            d,
            font,
            &line,
            body.x + 12.0,
            y,
            metric::UI_FONT_CAPTION,
            tint,
        );
        y += 17.0;
    }
    *cursor += height + 8.0;
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
///
/// **Both columns are ellipsized, independently.** The label column is as
/// likely to be a catalog string as the value column — the OpenRouter list draws
/// the model *id* as its label — and a label measured against nothing printed
/// straight through the value beside it (AP3-R S7).
fn field_line(
    d: &mut RaylibDrawHandle<'_>,
    font: &musializer_runtime::font::UiFonts,
    body: UiRect,
    cursor: &mut f32,
    label: &str,
    value: &str,
    tint: Color,
) {
    let offset = 186.0f32.min(body.width * 0.4);
    widgets::draw_text(
        d,
        font,
        &ellipsize(
            font,
            label,
            (offset - 8.0).max(16.0),
            metric::UI_FONT_CAPTION,
        ),
        body.x,
        *cursor,
        metric::UI_FONT_CAPTION,
        color::ui_muted(),
    );
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

/// The longest run of characters any externally-sourced string may contribute
/// to a drawn row.
///
/// A bound rather than a guess, and deliberately generous: a model id of 160
/// characters is already absurd, and every column ellipsizes against the face
/// afterwards anyway. The point of the cap is that
/// [`sanitize_display`]'s cost is bounded by its *output*, so a 4000-character
/// name cannot make a row measurement walk 4000 glyphs on every frame.
pub const DISPLAY_CAP: usize = 160;

/// Whether a character reorders text without drawing anything.
///
/// `char::is_control` does not cover these: they are format characters, so a
/// right-to-left override survives a control-character strip and silently
/// reverses everything after it in the row. The reviewer's fixture carries one
/// (`b/rtl-\u{202e}reversed\u{202c}-and--nul`), which is why they are named here
/// rather than assumed absent.
fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// The one filter every catalog-, Codex- and doctor-derived string goes through
/// before it is measured or drawn (E6, AP3-R S7).
///
/// Three things, in one place because they are one rule — *a string from
/// outside this application is data, not layout*:
///
/// - **Control and bidi-format characters are dropped.** A newline in a model id
///   made the row below it collide with this one, and a right-to-left override
///   reverses the rest of the line including the label beside it.
/// - **Whitespace runs collapse to one space**, so a name padded with forty tabs
///   occupies one column rather than the whole row.
/// - **The length is hard-capped with an ellipsis.** A 1300-character id
///   overprinted the row it was in; the column ellipsizer alone could not save it
///   because the *measurement* of the untruncated string is what cost the frame.
///
/// It is deliberately not a validator. Nothing here refuses a string — a catalog
/// entry with a hostile id still appears, it simply cannot rearrange the page.
#[must_use]
pub fn sanitize_display(text: &str) -> String {
    let mut out = String::new();
    let mut drawn = 0usize;
    let mut pending_space = false;
    for character in text.chars() {
        // A bidi override is *deleted*, not turned into a space: it sits between
        // two halves of a word that were meant to be one, and inserting a
        // separator would change the text rather than only its direction.
        if is_bidi_control(character) {
            continue;
        }
        if character.is_control() || character.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && drawn > 0 {
            if drawn >= DISPLAY_CAP {
                out.push('\u{2026}');
                return out;
            }
            out.push(' ');
            drawn += 1;
        }
        pending_space = false;
        if drawn >= DISPLAY_CAP {
            out.push('\u{2026}');
            return out;
        }
        out.push(character);
        drawn += 1;
    }
    out
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
        self.draw_banners(d, font, body, cursor);
        heading(
            d, font, body, cursor,
            Section::Routing.heading(),
            "One row per task contract. Every change applies to the next job \u{2014} a job already \
             running keeps the routes it started with.",
        );

        // Profile and the experimental gate.
        let options = profile_options(&self.draft);
        let active = self.draft.active_profile.clone();
        let profile_label = format!(
            "Profile: {}",
            if active == RECOMMENDED_PROFILE {
                "Recommended (built in)".to_string()
            } else {
                format!("{} (per-task overrides)", sanitize_display(&active))
            }
        );
        let profile_box = UiRect::new(body.x, *cursor, 250.0f32.min(body.width), CONTROL_HEIGHT);
        let (target, switches) = override_target(&self.draft);
        let profile_tip = format!(
            "{} profile(s) in this file, plus the built-in Recommended one. Edits go to {}{}.",
            options.len(),
            sanitize_display(&target),
            if switches {
                ", which this would make active"
            } else {
                ""
            }
        );
        let profile_enabled = self.profile_picker_enabled();
        if self.picker(
            d,
            font,
            20,
            FOCUS_ROUTING_PROFILE,
            profile_box,
            &profile_label,
            if profile_enabled {
                &profile_tip
            } else if self.settings_editable() {
                "This file carries no profile of its own yet, so the built-in Recommended one is \
                 the only choice. Changing any route below creates one."
            } else {
                "Disabled: the settings file failed to load, so there is no profile list to \
                 choose from."
            },
            profile_enabled,
        ) {
            // Every profile in the file, plus `recommended` — not a two-state
            // flip between `recommended` and `custom`, which could not reach a
            // profile called anything else (AP3-R S2).
            if let Some(next) = cycle(&options, Some(&active)) {
                self.draft.active_profile = next;
            }
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
            self.experimental_toggle_enabled(),
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
        // Defect B. A contract the filter leaves with nothing to offer says so
        // in a full-width sentence, because the MODEL column has 202 px and the
        // sentence a user has to act on does not fit in it. The cell keeps
        // showing the configured model — hiding it would misreport the file —
        // and this is where the row explains why that model is not a choice.
        let starved = self.contracts_without_an_offer();
        self.last_starved_band = None;
        if !starved.is_empty() {
            let named = starved
                .iter()
                .map(|contract| contract.token().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let top = *cursor;
            paragraph(
                d,
                font,
                body,
                cursor,
                &format!(
                    "{NO_OFFERABLE_MODEL}: {named}. The MODEL cell still shows what your settings \
                     configure, marked \"not offered\" \u{2014} it is the configuration, not an \
                     offer.",
                ),
                color::ui_warning(),
            );
            self.last_starved_band = Some(UiRect::new(body.x, top, body.width, *cursor - top));
            *cursor += 4.0;
        }
        // The boundary legend (AP3-R S9). The BOUNDARY column is three tokens
        // nobody outside this document has met, and the ladder they belong to is
        // the single most consequential thing on the page.
        paragraph(
            d,
            font,
            body,
            cursor,
            "BOUNDARY: local-only \u{2014} nothing leaves this machine, no socket is opened. \
             text-leaves-machine \u{2014} derived text or JSON leaves, never audio. \
             audio-leaves-machine \u{2014} audio bytes leave, and every job asks first.",
            color::ui_muted(),
        );
        *cursor += 4.0;
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
    ///
    /// The CONTRACT column is 150 px rather than the original 104 because it now
    /// carries the plain-language name over the `TC-*` token (AP3-R S9), and
    /// "Independent timing verification" needs the room. Widening it moves the
    /// narrowest body the matrix fits in, which is why
    /// [`ROUTING_MATRIX_DIALOG_WIDTH`] moved with it — the two constants are
    /// tied together by a test.
    fn routing_columns(width: f32) -> [f32; 6] {
        let fixed = [150.0f32, 132.0, 118.0, 106.0, 96.0];
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

            // The plain-language name is the primary text and the stable token
            // is the dimmed second line (AP3-R S9). The token is what files,
            // snapshots and cache acceptance are keyed on, so it stays on
            // screen — but "TC-COARSE" was the *only* thing the row said it did.
            widgets::draw_text(
                d,
                font,
                &ellipsize(
                    font,
                    contract.human_label(),
                    columns[0],
                    metric::UI_FONT_CAPTION,
                ),
                x,
                row_y + 1.0,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            let token = if resolved.origin == RouteOrigin::Override {
                format!("{}*", contract.token())
            } else {
                contract.token().to_string()
            };
            widgets::draw_text(
                d,
                font,
                &token,
                x,
                row_y + 16.0,
                MARKER_FONT_SIZE,
                color::ui_muted(),
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
            self.readiness_badge(
                d,
                font,
                900 + index as u32,
                UiRect::new(x, row_y + 3.0, columns[5], 20.0),
                &readiness,
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
            let token = if resolved.origin == RouteOrigin::Override {
                format!("{}*", contract.token())
            } else {
                contract.token().to_string()
            };
            let available = (body.width - 108.0).max(40.0);
            widgets::draw_text(
                d,
                font,
                &ellipsize(
                    font,
                    contract.human_label(),
                    available,
                    metric::UI_FONT_CAPTION,
                ),
                body.x,
                *cursor,
                metric::UI_FONT_CAPTION,
                color::ui_ink(),
            );
            self.readiness_badge(
                d,
                font,
                900 + index as u32,
                UiRect::new(
                    body.x + body.width - 96.0,
                    *cursor - 3.0,
                    96.0f32.min(body.width),
                    18.0,
                ),
                &readiness,
            );
            *cursor += 15.0;
            widgets::draw_text(
                d,
                font,
                &format!("{token} \u{00b7} {}", resolved.boundary().token()),
                body.x,
                *cursor,
                MARKER_FONT_SIZE,
                color::ui_muted(),
            );
            *cursor += 16.0;
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
        let enabled = self.route_cell_enabled(contract);
        let label = if contract.is_locked() {
            "locked".to_string()
        } else {
            resolved.route.as_ref().map_or_else(
                || "choose".to_string(),
                |route| route.route_type.token().to_string(),
            )
        };
        // The disabled reasons are as specific as the enabled one, because a
        // disabled control's tooltip is the only place it can explain itself
        // (AP3-R S5).
        let tip = if enabled {
            format!(
                "{} ({}): {} eligible route type(s) \u{00b7} caps at {}",
                contract.human_label(),
                contract.token(),
                eligible.len(),
                contract.max_boundary().token()
            )
        } else if !self.settings_editable() {
            "Disabled: the settings file failed to load, so this is showing the built-in default \
             rather than your file."
                .to_string()
        } else if contract.is_locked() {
            format!(
                "{} is the built-in deterministic analyzer and is not user-routable (§7).",
                contract.token()
            )
        } else {
            format!(
                "{} has exactly one eligible route type ({}), so there is nothing to switch to.",
                contract.token(),
                eligible
                    .first()
                    .map_or("none", |route_type| route_type.token())
            )
        };
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
        let label = sanitize_display(&resolved.model_label());
        let status = resolved
            .route
            .as_ref()
            .and_then(|route| route.model_id.as_deref())
            .map(|id| suitability::status(id, contract));
        let enabled = self.model_cell_enabled(contract);
        let show_experimental = self.draft.catalog.show_experimental;
        let with_experimental = self.model_options_with_experimental(contract).len();
        let tip = if !enabled && !self.settings_editable() {
            "Disabled: the settings file failed to load, so this is showing the built-in default \
             rather than your file."
                .to_string()
        } else if options.is_empty() && with_experimental > 0 && !contract.is_locked() {
            // Defect B: the picker offers nothing, and the cell is still showing
            // the configured model. Both facts, in that order, because the
            // second one is what makes the first look like a bug.
            format!(
                "{NO_OFFERABLE_MODEL}. {} is what your settings configure for {}, and it is \
                 experimental, so with the toggle off no picker offers it. It is shown because \
                 hiding it would misreport your configuration.",
                label,
                contract.token()
            )
        } else if !enabled && !contract.is_locked() {
            // AP3-R S8: a picker whose only option is what is already selected
            // is disabled — but *why* it is the only option matters. When the
            // experimental filter is what emptied it, the control names the
            // toggle that would refill it rather than looking broken.
            if with_experimental > options.len() {
                format!(
                    "Only {} model is offered for {}. {} more become available with \
                     \"Show experimental\" turned on, above.",
                    options.len(),
                    contract.token(),
                    with_experimental - options.len()
                )
            } else {
                format!(
                    "{} is the only model offered for {}. A model measured and failed is never \
                     offered, whatever this is set to.",
                    label,
                    contract.token()
                )
            }
        } else {
            // Every one of these says whether it is describing the *configured*
            // model or the *offer*, because defect B was exactly the two being
            // conflated.
            match status {
                Some(Suitability::Recommended) => format!(
                    "Configured: {label} \u{2014} recommended for {} on this repository's own \
                     benchmark. {} model(s) offered.",
                    contract.token(),
                    options.len()
                ),
                Some(Suitability::Experimental) if show_experimental => format!(
                    "Configured: {label} \u{2014} experimental, no Musializer benchmark for {}. \
                     It is offered because \"Show experimental\" is on.",
                    contract.token()
                ),
                Some(Suitability::Experimental) => format!(
                    "Configured: {label} \u{2014} experimental, so with \"Show experimental\" off \
                     it is not offered by any picker. {} other model(s) are.",
                    options.len()
                ),
                Some(Suitability::Unsupported) => format!(
                    "Configured: {label} \u{2014} measured and failed for {}; it is never offered, \
                     whatever the toggle says.",
                    contract.token()
                ),
                None => format!("{} models offered for {}", options.len(), contract.token()),
            }
        };
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
        //
        // The text is `model_marker`'s, not `status.token()`'s: the bare token
        // read as a claim about the offer, which is the half of defect B a
        // capture could see.
        if let Some(marker) = status.and_then(|status| model_marker(status, show_experimental)) {
            widgets::draw_text(
                d,
                font,
                &ellipsize(font, marker, boundary.width - 2.0, MARKER_FONT_SIZE),
                boundary.x + 2.0,
                boundary.y + boundary.height + 1.0,
                MARKER_FONT_SIZE,
                if status == Some(Suitability::Unsupported) {
                    color::ui_danger()
                } else {
                    color::ui_warning()
                },
            );
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
        let enabled = self.fallback_cell_enabled(contract);
        let label = resolved.route.as_ref().map_or_else(
            || "\u{2014}".to_string(),
            |route| route.fallback.token().to_string(),
        );
        let tip = if enabled {
            format!(
                "{}: {}",
                contract.token(),
                allowed
                    .iter()
                    .map(|policy| policy.token())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if !self.settings_editable() {
            "Disabled: the settings file failed to load, so this is showing the built-in default \
             rather than your file."
                .to_string()
        } else if resolved.route.is_none() {
            // AP3-R S6: this was drawn enabled and did nothing. A fallback is a
            // policy *for a route*, so with no route there is nothing to attach
            // one to.
            format!(
                "Choose a route first: {} has no recommended route, so there is nothing for a \
                 fallback policy to apply to.",
                contract.token()
            )
        } else {
            format!(
                "{} allows only the {} fallback policy.",
                contract.token(),
                allowed.first().map_or("none", |policy| policy.token())
            )
        };
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
        self.draw_banners(d, font, body, cursor);
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
                    &if report.schema_version.is_empty() {
                        "not reported".to_string()
                    } else {
                        sanitize_display(&report.schema_version)
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
                                &sanitize_display(&identity.state),
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
                                    &value.as_deref().map_or_else(
                                        || "not reported".to_string(),
                                        sanitize_display,
                                    ),
                                    if value.is_some() {
                                        color::ui_ink()
                                    } else {
                                        color::ui_muted()
                                    },
                                );
                            }
                            if let Some(remediation) = &identity.remediation {
                                paragraph(
                                    d,
                                    font,
                                    body,
                                    cursor,
                                    &sanitize_display(remediation),
                                    color::ui_warning(),
                                );
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
            true,
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
        let doctor_enabled = self.doctor_enabled();
        if self.picker(
            d,
            font,
            32,
            FOCUS_MODELS_DOCTOR,
            doctor_box,
            "Run doctor",
            if doctor_enabled {
                "Runs tools/musializer_doctor.py --json and reads the runtime identities from it"
            } else {
                "Disabled while another background job is running in this dialog."
            },
            doctor_enabled,
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
        self.draw_banners(d, font, body, cursor);
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
        self.draw_codex_executable(d, font, body, cursor);

        subheading(d, font, body, cursor, "Discovered models");
        // §5 rule 6 is unchanged by defect C: whatever discovery concludes, the
        // *model* list falls back to exactly `Codex default` and never to a
        // guessed id. Finding the binary differently does not license inventing
        // what it can run.
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
                            sanitize_display(&model.id),
                            if model.is_default { " (default)" } else { "" }
                        ),
                        &format!(
                            "{} \u{00b7} efforts: {} \u{00b7} default: {}",
                            sanitize_display(if model.display_name.is_empty() {
                                model.id.as_str()
                            } else {
                                model.display_name.as_str()
                            }),
                            if efforts.is_empty() {
                                "not reported".to_string()
                            } else {
                                sanitize_display(&efforts.join("/"))
                            },
                            sanitize_display(
                                model
                                    .default_reasoning_effort
                                    .as_deref()
                                    .unwrap_or("not reported")
                            ),
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
            let enabled = self.codex_effort_enabled(contract);
            let effort = resolved
                .route
                .as_ref()
                .and_then(|route| route.reasoning_effort);
            widgets::draw_text(
                d,
                font,
                &format!("{} ({})", contract.human_label(), contract.token()),
                body.x,
                *cursor + 6.0,
                MARKER_FONT_SIZE,
                color::ui_ink(),
            );
            let boundary = UiRect::new(
                body.x + 232.0,
                *cursor,
                180.0f32.min((body.width - 232.0).max(0.0)),
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
            let tip = if enabled {
                "minimal / low / medium / high".to_string()
            } else if !self.settings_editable() {
                "Disabled: the settings file failed to load.".to_string()
            } else {
                format!(
                    "{} is not routed through Codex, so it has no reasoning effort to set. \
                     Change its ROUTE to codex on the Routing page first.",
                    contract.token()
                )
            };
            if self.picker(
                d,
                font,
                600 + index as u32,
                FOCUS_CODEX_EFFORT_BASE + index as ControlId,
                boundary,
                &label,
                &tip,
                enabled,
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
        let refresh_enabled = self.codex_refresh_enabled();
        let refresh = UiRect::new(body.x, *cursor, 190.0f32.min(body.width), CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            610,
            FOCUS_CODEX_REFRESH,
            refresh,
            "Refresh Codex models",
            if refresh_enabled {
                "Runs tools/codex_model_discovery.py, which starts a short-lived `codex app-server`"
            } else if self.codex_binary().is_none() {
                "Disabled: `codex` could not be found, so there is nothing to ask. The Executable \
                 block above lists everywhere this looked."
            } else {
                "Disabled while another background job is running in this dialog."
            },
            refresh_enabled,
        ) {
            self.start_refresh(RefreshKind::Codex);
        }
        *cursor += CONTROL_HEIGHT + 8.0;
    }
}

impl AssistSettingsDialog {
    /// The Executable block of the Codex section.
    ///
    /// Defect C's user-facing half. Three things have to be on screen and only
    /// one of them was: **where** it is, **how** it was found, and — when it was
    /// not — **where this looked**, because "not on PATH" sent the operator to
    /// fix a `PATH` that was already correct in every shell they use.
    fn draw_codex_executable(
        &mut self,
        d: &mut RaylibDrawHandle<'_>,
        font: &musializer_runtime::font::UiFonts,
        body: UiRect,
        cursor: &mut f32,
    ) {
        let Some(discovery) = self.codex_discovery.clone() else {
            field_line(
                d,
                font,
                body,
                cursor,
                "codex",
                "looking\u{2026}",
                color::ui_muted(),
            );
            return;
        };
        match &discovery.outcome {
            discover::Outcome::Found { path, method } => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "codex",
                    &sanitize_display(&path.display().to_string()),
                    color::ui_success(),
                );
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "found by",
                    &format!("{} \u{2014} {}", method.token(), method.explanation()),
                    color::ui_muted(),
                );
                if *method == discover::Method::LoginShell {
                    // The one hit worth explaining unprompted: it means this
                    // process's own environment is missing something the user's
                    // shell has, which is normal under a desktop entry and
                    // surprising if you have not met it.
                    paragraph(
                        d,
                        font,
                        body,
                        cursor,
                        "This process's own PATH does not carry codex \u{2014} it was found by \
                         asking your login shell. That is expected when Musializer is started \
                         from a desktop entry rather than a terminal.",
                        color::ui_muted(),
                    );
                }
            }
            discover::Outcome::OverrideMissing { path, reason } => {
                // Loud, and no fallback. The user typed this path.
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "codex",
                    &format!(
                        "{}: {reason}",
                        sanitize_display(&path.display().to_string())
                    ),
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "The codex_bin setting in assist.json names this path. Discovery is not \
                     attempted while it is set: an explicit setting that quietly fell back to \
                     something else would look like it does nothing. Fix the path or remove the \
                     setting.",
                    color::ui_warning(),
                );
            }
            discover::Outcome::NotFound => {
                let still_looking = self.codex_probe.is_some();
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "codex",
                    if still_looking {
                        "looking\u{2026}"
                    } else {
                        "not found"
                    },
                    if still_looking {
                        color::ui_muted()
                    } else {
                        color::ui_danger()
                    },
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    if still_looking {
                        "Not on this process's PATH or in a common install location. Still asking \
                         your login shell."
                    } else if discovery.consulted_subprocess {
                        "Install Codex, or set local_runtimes.codex_bin in assist.json to its \
                         path. Musializer never reads Codex's authentication files."
                    } else {
                        "Install Codex, or set local_runtimes.codex_bin in assist.json to its \
                         path. The login-shell lookup was switched off for this run, so this is \
                         not a complete search."
                    },
                    color::ui_muted(),
                );
                // Where it looked. Printed, not counted: a list is what a user
                // can compare against where they know the binary is.
                //
                // Sanitized **per entry** and joined afterwards, not the other
                // way round. `sanitize_display` caps its output at
                // `DISPLAY_CAP`, so sanitizing the joined string turned twelve
                // directories into the first one and an ellipsis — which a
                // capture caught and no test would have, because the function
                // was doing exactly what it says.
                subheading_row(d, font, body, cursor, "Looked in");
                let looked = discovery
                    .searched
                    .iter()
                    .map(|entry| sanitize_display(entry))
                    .collect::<Vec<_>>()
                    .join(" \u{00b7} ");
                paragraph(d, font, body, cursor, &looked, color::ui_muted());
            }
        }
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
        self.draw_banners(d, font, body, cursor);
        heading(
            d, font, body, cursor,
            Section::OpenRouter.heading(),
            "The one remote provider. Its key lives in a 0600 file under your config directory \u{2014} \
             never in a .musi project, a preference file, a log, or a command line.",
        );

        subheading(d, font, body, cursor, "Connection");
        // The path the key is read from and written to, here rather than only on
        // the Privacy page (AP3-R S9): every remediation on this page — chmod,
        // Forget, "there is nowhere to store a key" — names a file the section
        // otherwise never showed.
        field_line(
            d,
            font,
            body,
            cursor,
            "Credentials file",
            &self.credentials_path.as_ref().map_or_else(
                || "no per-user configuration directory".to_string(),
                |path| path.display().to_string(),
            ),
            color::ui_muted(),
        );
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
            CredentialState::Unusable { kind, path, entry } => {
                field_line(
                    d,
                    font,
                    body,
                    cursor,
                    "Key",
                    &match (kind, &entry) {
                        (CredentialFault::Io, _) => {
                            "unusable \u{2014} could not be read".to_string()
                        }
                        (CredentialFault::NotJson, _) => {
                            "unusable \u{2014} the file is not JSON".to_string()
                        }
                        (CredentialFault::TooLarge, _) => {
                            "unusable \u{2014} over the size cap".to_string()
                        }
                        (CredentialFault::Schema, Some(entry)) => {
                            format!("unusable \u{2014} the {entry} entry is malformed")
                        }
                        (CredentialFault::Schema, None) => {
                            "unusable \u{2014} not a credentials store of this schema".to_string()
                        }
                    },
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    &format!("{path}: {}", kind.remediation()),
                    color::ui_danger(),
                );
                paragraph(
                    d,
                    font,
                    body,
                    cursor,
                    "The file is left exactly as it is. Musializer does not rewrite a credentials \
                     file it could not read, for the same reason it does not repair a loose one.",
                    color::ui_muted(),
                );
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
        let busy_reason = "Disabled while another background job is running in this dialog.";
        let button_width = 118.0f32.min((body.width - 24.0) / 4.0);
        let mut x = body.x;
        let test_enabled = self.key_test_enabled();
        let test = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            700,
            FOCUS_KEY_TEST,
            test,
            "Test",
            if test_enabled {
                "GET /api/v1/key \u{2014} read-only, non-inference, spends no credits"
            } else if busy {
                busy_reason
            } else {
                "Disabled: there is no key to test. Type or paste one above, or store one first."
            },
            test_enabled,
        ) {
            self.start_key_test();
        }
        x += button_width + 8.0;
        let replace_enabled = self.key_replace_enabled();
        let replace = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            701,
            FOCUS_KEY_REPLACE,
            replace,
            "Replace",
            if replace_enabled {
                "Commits the typed key. Enabled once a Test has succeeded."
            } else if busy {
                busy_reason
            } else if has_pending {
                "Disabled until a Test succeeds for the key typed above (§3). Save untested \
                 commits it without one."
            } else {
                "Disabled: nothing has been typed in the field above."
            },
            replace_enabled,
        ) {
            self.commit_key();
        }
        x += button_width + 8.0;
        let untested_enabled = self.key_untested_enabled();
        let untested = UiRect::new(x, *cursor, button_width + 30.0, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            702,
            FOCUS_KEY_SAVE_UNTESTED,
            untested,
            "Save untested",
            if untested_enabled {
                "Commits without a Test. The dialog will not claim the key works."
            } else if busy {
                busy_reason
            } else {
                "Disabled: nothing has been typed in the field above."
            },
            untested_enabled,
        ) {
            self.commit_key();
        }
        x += untested.width + 8.0;
        let forget_enabled = self.key_forget_enabled();
        let forget = UiRect::new(x, *cursor, button_width, CONTROL_HEIGHT);
        if self.picker(
            d,
            font,
            703,
            FOCUS_KEY_FORGET,
            forget,
            "Forget",
            if forget_enabled {
                "Removes this provider's entry and rewrites the file; other providers are untouched"
            } else if busy {
                busy_reason
            } else {
                "Disabled: there is no stored key in the credentials file to forget. A session-only \
                 key disappears when Musializer exits."
            },
            forget_enabled,
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
                let source = sanitize_display(&cache.source_url);
                let filters: Vec<String> = cache
                    .filters
                    .iter()
                    .map(|(key, value)| {
                        format!("{}={}", sanitize_display(key), sanitize_display(value))
                    })
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
                    // Every string below is catalog data, so every one of them
                    // goes through `sanitize_display` before it is measured
                    // (E6, AP3-R S7). The reviewer's fixture carries a
                    // 1322-character id, a name with an embedded newline and a
                    // right-to-left override; all three used to reach the
                    // renderer verbatim.
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
                        (Some(audio), _, _) => format!("audio ${}", sanitize_display(audio)),
                        (None, Some(prompt), Some(completion)) => format!(
                            "in ${} / out ${}",
                            sanitize_display(prompt),
                            sanitize_display(completion)
                        ),
                        (None, Some(prompt), None) => {
                            format!("in ${}", sanitize_display(prompt))
                        }
                        _ => "price not reported".to_string(),
                    };
                    field_line(
                        d,
                        font,
                        body,
                        cursor,
                        &sanitize_display(&model.id),
                        &format!(
                            "{} \u{00b7} {modalities} \u{00b7} {context} \u{00b7} {price} \u{00b7} {}",
                            if model.name.is_empty() {
                                "name not reported".to_string()
                            } else {
                                sanitize_display(&model.name)
                            },
                            // Defect B applies here too. This is a *cache
                            // listing*, not a picker, so the row keeps naming
                            // the verdict — but a bare `experimental` under
                            // `Show experimental: off` reads as an offer, and
                            // the whole ruling is that the two are different
                            // claims. Whether the toggle would offer this row is
                            // the second half.
                            if status.offered(self.draft.catalog.show_experimental) {
                                status.token().to_string()
                            } else {
                                format!("{} \u{00b7} not offered", status.token())
                            }
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
            true,
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
            true,
        ) {
            self.draft.catalog.refresh_on_open = !refresh_on_open;
        }
        x += toggle_width + 8.0;
        let catalog_refresh_enabled = self.catalog_refresh_enabled();
        if self.picker(
            d,
            font,
            712,
            FOCUS_CATALOG_REFRESH,
            UiRect::new(x, *cursor, toggle_width, CONTROL_HEIGHT),
            "Refresh now",
            if catalog_refresh_enabled {
                "Runs tools/provider_catalog.py. Fetching a public catalog still discloses your IP \
                 address, which is why it is never disguised as an offline action."
            } else if busy {
                busy_reason
            } else {
                "Disabled: Network access is off, which is what stops this from opening a socket. \
                 Turn it on beside this button."
            },
            catalog_refresh_enabled,
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
        self.draw_banners(d, font, body, cursor);
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
            let is_remote = self.zdr_enabled(contract);
            widgets::draw_text(
                d,
                font,
                &format!("{} ({})", contract.human_label(), contract.token()),
                body.x,
                *cursor + 6.0,
                MARKER_FONT_SIZE,
                color::ui_ink(),
            );
            let boundary = UiRect::new(
                body.x + 232.0,
                *cursor,
                220.0f32.min((body.width - 232.0).max(0.0)),
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
                    true,
                )
            } else {
                self.picker(
                    d,
                    font,
                    800 + index as u32,
                    FOCUS_PRIVACY_ZDR_BASE + index as ControlId,
                    boundary,
                    "local route \u{00b7} nothing leaves",
                    if self.settings_editable() {
                        "This contract resolves to a local route, so no data leaves this machine \
                         and there is no provider policy to set. Route it through OpenRouter on \
                         the Routing page to get one."
                    } else {
                        "Disabled: the settings file failed to load."
                    },
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
            &sanitize_display(&self.draft.active_profile),
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
                &format!(
                    "{} ({})",
                    resolved.contract.human_label(),
                    resolved.contract.token()
                ),
                &format!(
                    "{} \u{00b7} {} \u{00b7} {} \u{00b7} fallback {} \u{00b7} {} \u{00b7} {}",
                    resolved.route_label(),
                    sanitize_display(&resolved.model_label()),
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

        // AP3-R S4. Neither hand-drawn field has a caret, so Home and End have
        // no text meaning to take here — and a scrollable modal with no keyboard
        // way down it is a modal a keyboard user cannot finish reading. The
        // values are clamped in `draw_body` against the height it measured, so
        // an over-large End is bounded by the same expression the wheel is.
        let page = (self.last_body_height - 48.0).max(96.0);
        if repeat(d, Key::KEY_PAGE_DOWN) {
            self.scroll += page;
        }
        if repeat(d, Key::KEY_PAGE_UP) {
            self.scroll = (self.scroll - page).max(0.0);
        }
        if d.is_key_pressed(Key::KEY_HOME) {
            self.scroll = 0.0;
        }
        if d.is_key_pressed(Key::KEY_END) {
            self.scroll = self.content_height;
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

/// The half-sentence a credential status line ends with, depending on whether
/// the settings file was rewritten with it.
fn settings_follow_up(persisted: bool) -> &'static str {
    if persisted {
        " The settings file records its fingerprint."
    } else {
        " Save to record its fingerprint in the settings file."
    }
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
                let record = musializer_core::assist::settings::CredentialRecord {
                    mode: CredentialMode::File,
                    lookup_id: lookup,
                    fingerprint: Some(fingerprint.clone()),
                    label: label.clone(),
                };
                let persisted = self.record_credential(record);
                self.credentials = CredentialState::File { fingerprint, label };
                self.key_tested_ok = false;
                self.status = Some((
                    format!(
                        "Key stored in {} (mode 0600).{}",
                        path.display(),
                        settings_follow_up(persisted)
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
                let persisted = self.record_credential(
                    musializer_core::assist::settings::CredentialRecord::default(),
                );
                self.key_test = None;
                self.key_tested_ok = false;
                self.status = Some((
                    format!(
                        "Key forgotten. Other providers' entries were left byte-identical.{}",
                        settings_follow_up(persisted)
                    ),
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

    /// Records a credential operation in **both** settings stores at once
    /// (AP3-R S13).
    ///
    /// The defect this replaces: `commit_key` and `forget_key` wrote the
    /// credentials file immediately and put the matching metadata only in the
    /// *draft*. So Forget followed by Escape-discard threw the draft away and
    /// left `saved` — the record the rest of the dialog and the next Save read —
    /// still claiming `mode: file` with the fingerprint of a key that no longer
    /// existed anywhere. A credential operation is not an edit that can be
    /// discarded: it already happened, on disk, in another file.
    ///
    /// So it lands in `saved` and `draft` together, which also means it never
    /// moves the dirty flag. The settings *file* is rewritten too when that is
    /// safe — there were no other unsaved edits to sweep along with it, and the
    /// file on disk is one this dialog could read. Otherwise it is left alone and
    /// the status line says a Save is still owed, because silently writing a
    /// draft the user is still editing is the worse failure.
    ///
    /// Returns whether the settings file itself was rewritten.
    fn record_credential(
        &mut self,
        record: musializer_core::assist::settings::CredentialRecord,
    ) -> bool {
        let was_clean = !self.is_dirty();
        self.draft.credentials.openrouter = record.clone();
        self.saved.credentials.openrouter = record;
        let writable = matches!(
            self.settings_source,
            SettingsSource::Loaded | SettingsSource::Absent
        );
        if !was_clean || !writable {
            return false;
        }
        let Some(path) = self.settings_path.clone() else {
            return false;
        };
        if files::save_settings(&path, &self.saved).is_ok() {
            self.settings_source = SettingsSource::Loaded;
            return true;
        }
        false
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
                // The resolved path, not the bare name. The tool would spawn
                // `codex` from *its* PATH, which is this process's PATH — the
                // exact environment defect C is about. Handing it the answer we
                // already found is what makes discovery worth doing.
                if let Some(path) = self.codex_binary() {
                    command.arg("--codex-bin").arg(path);
                }
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
    // `Boundary` is used only by the tests now that route resolution moved to
    // `core::assist::execution`; importing it here rather than at module scope
    // keeps the non-test build free of an unused import.
    use musializer_core::assist::contracts::Boundary;

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

    /// Defect A. The section list is inset by the same gap on both sides, in
    /// **both** arrangements.
    ///
    /// It shipped at 0 px against the divider and 14 px against the dialog's
    /// border, which reads as the buttons having fallen off the rail. Asserted
    /// as a sweep rather than at one window, because the tabs arrangement
    /// derives its cell width by division and a rounding change there would
    /// break the right-hand gap alone.
    #[test]
    fn the_section_list_is_inset_equally_on_both_sides() {
        let mut width = 1600.0f32;
        let mut rail_windows = 0;
        let mut tab_windows = 0;
        while width >= 640.0 {
            let layout = DialogLayout::of((width, 800.0));
            let (left, right) = layout.section_gaps();
            assert!(
                (left - right).abs() <= 0.5,
                "{width}px ({}): left gap {left} against right gap {right}",
                if layout.rail_on_left { "rail" } else { "tabs" }
            );
            assert!(left > 0.0, "{width}px: the list touches the dialog border");
            if layout.rail_on_left {
                rail_windows += 1;
                // And the gap is the dialog's own padding, not some third
                // number that happens to match on both sides.
                assert!((left - DIALOG_PADDING).abs() <= 0.5, "{width}px: {left}");
                // The rail's right edge stops short of the divider it is
                // measured against, which is the thing the operator saw.
                assert!(layout.tabs[0].x + layout.tabs[0].width < layout.content.x);
            } else {
                tab_windows += 1;
            }
            width -= 4.0;
        }
        assert!(
            rail_windows > 0 && tab_windows > 0,
            "one arrangement went untested"
        );
    }

    /// The routing matrix stacks rather than clipping, and its columns always
    /// sum to the body width.
    #[test]
    fn the_routing_columns_fill_the_body_without_overflowing_it() {
        for width in [ROUTING_MATRIX_BODY_WIDTH, 760.0, 860.0, 1010.0] {
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
            // Close and the five section tabs are always reachable; everything
            // else depends on state, and since AP3-R S5 a control drawn
            // disabled is not a tabstop. A default dialog with no key, no
            // catalog and no `codex` on PATH legitimately offers little else.
            assert!(total >= 6, "{section:?} offers only {total} controls");
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
        // Defect B, and the reversal it records. A selected model the filter
        // drops is **not** put back: the escape hatch that used to do it is what
        // made `Show experimental: off` display an experimental model with an
        // orange marker, and a toggle its own row contradicts is worse than no
        // toggle. The configuration is still shown — by the cell, in words, not
        // by smuggling it into the offer list.
        let with_selected = model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            false,
            Some("xiaomi/mimo-v2.5"),
        );
        assert!(
            with_selected.is_empty(),
            "the selected experimental model was offered with the toggle off: {with_selected:?}"
        );
        // Turn the toggle on and the very same call offers it.
        assert!(model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            true,
            Some("xiaomi/mimo-v2.5"),
        )
        .contains(&"xiaomi/mimo-v2.5".to_string()));
        // A selected id the *catalog* has never heard of is a different case and
        // keeps its escape hatch, so a stale cache does not strand a picker —
        // but only while the overlay would offer it.
        let stale = model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            true,
            Some("acme/vanished"),
        );
        assert_eq!(stale.first().map(String::as_str), Some("acme/vanished"));
        assert!(model_options(
            ContractId::Semantic,
            RouteType::OpenRouter,
            Some(&catalog),
            None,
            false,
            Some("acme/vanished"),
        )
        .is_empty());
    }

    /// Defect B, the marker. Its words describe the configured model and say
    /// whether it is on offer; `recommended` is silent so the warnings read.
    #[test]
    fn the_marker_describes_the_configuration_and_not_the_offer() {
        assert_eq!(model_marker(Suitability::Recommended, false), None);
        assert_eq!(model_marker(Suitability::Recommended, true), None);
        assert_eq!(
            model_marker(Suitability::Experimental, true),
            Some("configured: experimental")
        );
        assert_eq!(
            model_marker(Suitability::Experimental, false),
            Some("not offered: experimental")
        );
        // Never offered at any toggle setting, so the toggle is not mentioned.
        for show in [false, true] {
            assert_eq!(
                model_marker(Suitability::Unsupported, show),
                Some("not offered: unsupported")
            );
        }
        // Every marker leads with the fact that survives ellipsis into 202 px.
        for marker in [
            model_marker(Suitability::Experimental, false),
            model_marker(Suitability::Unsupported, false),
        ]
        .into_iter()
        .flatten()
        {
            assert!(marker.starts_with("not offered"), "{marker}");
        }
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
            // Defect B: the offer and the configuration, as separate fields.
            "assist settings pickers: show-experimental=false",
            "TC-SEMANTIC{configured=xiaomi/mimo-v2.5 status=experimental \
             marker=\"not offered: experimental\" offers=[]}",
            "starved=[TC-SEMANTIC]",
        ] {
            assert!(joined.contains(key), "the report has no {key:?}:\n{joined}");
        }
        assert_eq!(lines.len(), 6);
        // Defect A: the chrome line appears once a frame has been drawn, and
        // carries the two gaps in pixels.
        dialog.last_geometry = Some(DialogLayout::of((1280.0, 720.0)));
        let drawn = dialog.describe().join("\n");
        assert!(
            drawn.contains("assist settings chrome: mode=rail")
                && drawn.contains("gap-left=14 gap-right=14 symmetric=true"),
            "the chrome line does not measure the section gaps:\n{drawn}"
        );
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

    // -- AP3-R -------------------------------------------------------------

    fn audio_catalog(ids: &[&str]) -> CatalogCache {
        CatalogCache {
            schema_version: "musializer.openrouter-catalog/v1".to_string(),
            source_url: String::new(),
            fetched_at_utc: None,
            filters: BTreeMap::new(),
            models: ids
                .iter()
                .map(|id| CatalogModel {
                    id: (*id).to_string(),
                    input_modalities: vec!["audio".to_string()],
                    output_modalities: vec!["text".to_string()],
                    ..CatalogModel::default()
                })
                .collect(),
        }
    }

    /// S1. A settings file that failed to load leaves the routing matrix
    /// showing built-in defaults, so every control on it is disabled and the
    /// ring skips all of them. The reviewer measured a corrupt-file routing
    /// capture as pixel-identical to a clean one; this is the model half of the
    /// fix, and the gate photographs the other half.
    #[test]
    fn a_failed_settings_load_disables_the_whole_routing_matrix() {
        let mut dialog = dialog();
        dialog.section = Section::Routing;
        let clean = dialog.focus_order();
        assert!(clean.contains(&FOCUS_ROUTING_EXPERIMENTAL));
        assert!(dialog.settings_editable());

        dialog.settings_source = SettingsSource::Error("the file is not valid JSON".to_string());
        assert!(!dialog.settings_editable());
        let broken = dialog.focus_order();
        assert!(!broken.contains(&FOCUS_ROUTING_EXPERIMENTAL));
        assert!(!broken.contains(&FOCUS_ROUTING_PROFILE));
        for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
            let index = index as ControlId;
            assert!(!dialog.route_cell_enabled(contract));
            assert!(!dialog.model_cell_enabled(contract));
            assert!(!dialog.fallback_cell_enabled(contract));
            assert!(!dialog.zdr_enabled(contract));
            for base in [FOCUS_ROUTE_BASE, FOCUS_MODEL_BASE, FOCUS_FALLBACK_BASE] {
                assert!(!broken.contains(&(base + index)));
            }
        }
        // The section tabs and Close stay: the error is escapable and the other
        // sections are still worth reading.
        assert!(broken.contains(&FOCUS_CLOSE));
        assert!(broken.contains(&FOCUS_TAB_BASE));
        // And it is in the report, on every section, not only on Privacy.
        for section in Section::ALL {
            dialog.section = section;
            assert!(
                dialog.describe()[0].contains("load-error=true"),
                "{section:?} does not report the failed load"
            );
        }
    }

    /// S2. The bug: `set_override` force-switched `active_profile` to `custom`,
    /// so a user editing their own `studio` profile got the edit written into a
    /// second profile and `studio` left in the file, never active again.
    ///
    /// The before/after is the whole assertion — every profile that was in the
    /// file is still in it, and the one that was active is still active.
    #[test]
    fn an_override_writes_to_the_active_profile_and_orphans_no_other() {
        let mut settings = AssistSettings {
            active_profile: "studio".to_string(),
            profiles: vec![
                Profile {
                    id: "studio".to_string(),
                    label: "Studio".to_string(),
                    routes: BTreeMap::new(),
                },
                Profile {
                    id: "live".to_string(),
                    label: "Live".to_string(),
                    routes: BTreeMap::new(),
                },
            ],
            ..AssistSettings::default()
        };
        let before: Vec<String> = settings.profiles.iter().map(|p| p.id.clone()).collect();

        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.fallback = FallbackPolicy::Ask;
        set_override(&mut settings, route);

        let after: Vec<String> = settings.profiles.iter().map(|p| p.id.clone()).collect();
        assert_eq!(before, after, "a profile was added, removed or reordered");
        assert_eq!(
            settings.active_profile, "studio",
            "the active profile was switched out from under the edit"
        );
        assert!(settings
            .profile("studio")
            .is_some_and(|profile| profile.routes.contains_key(&ContractId::Coarse)));
        assert!(settings
            .profile("live")
            .is_some_and(|profile| profile.routes.is_empty()));
        settings.validate().expect("still a valid record");
    }

    /// S2, the other half: the built-in profile is read-only, so an edit made
    /// while it is active copies into a user profile — reusing an existing one
    /// rather than inventing a second `custom` beside it.
    #[test]
    fn editing_the_built_in_profile_copies_into_a_user_profile() {
        let mut fresh = AssistSettings::default();
        assert_eq!(
            override_target(&fresh),
            (OVERRIDE_PROFILE_ID.to_string(), true)
        );
        let mut route = recommended_route(ContractId::Coarse).unwrap();
        route.fallback = FallbackPolicy::Ask;
        set_override(&mut fresh, route.clone());
        assert_eq!(fresh.active_profile, OVERRIDE_PROFILE_ID);
        assert_eq!(fresh.profiles.len(), 1);

        let mut existing = AssistSettings {
            active_profile: RECOMMENDED_PROFILE.to_string(),
            profiles: vec![Profile {
                id: "studio".to_string(),
                label: "Studio".to_string(),
                routes: BTreeMap::new(),
            }],
            ..AssistSettings::default()
        };
        assert_eq!(override_target(&existing), ("studio".to_string(), true));
        set_override(&mut existing, route);
        assert_eq!(existing.profiles.len(), 1, "a second profile was invented");
        assert_eq!(existing.active_profile, "studio");
        existing.validate().expect("still a valid record");
    }

    /// S2. The picker offers every profile the file carries, plus the built-in
    /// one — not a two-state flip that could never reach a third.
    #[test]
    fn the_profile_picker_lists_every_profile_in_the_file() {
        let settings = AssistSettings {
            active_profile: "studio".to_string(),
            profiles: vec![
                Profile {
                    id: "studio".to_string(),
                    label: "Studio".to_string(),
                    routes: BTreeMap::new(),
                },
                Profile {
                    id: "live".to_string(),
                    label: "Live".to_string(),
                    routes: BTreeMap::new(),
                },
            ],
            ..AssistSettings::default()
        };
        assert_eq!(
            profile_options(&settings),
            vec![
                RECOMMENDED_PROFILE.to_string(),
                "studio".to_string(),
                "live".to_string()
            ]
        );
        // And cycling reaches all three and comes back.
        let options = profile_options(&settings);
        let mut seen = vec!["studio".to_string()];
        let mut current = "studio".to_string();
        for _ in 0..3 {
            current = cycle(&options, Some(&current)).unwrap();
            seen.push(current.clone());
        }
        assert_eq!(seen[3], "studio", "the cycle did not return");
        assert!(seen.contains(&"live".to_string()));
        assert!(seen.contains(&RECOMMENDED_PROFILE.to_string()));
        assert_eq!(profile_options(&AssistSettings::default()).len(), 1);
    }

    /// S3. §5 invariant 4. The reviewer's fixture: a stored key, an OpenRouter
    /// route, and **no model chosen** reported `Ready`. Every required piece has
    /// to be present, and the badge names the first one that is not.
    #[test]
    fn readiness_names_the_first_missing_piece() {
        let mut dialog = dialog();
        let mut settings = AssistSettings::default();
        let mut route = recommended_route(ContractId::Semantic).unwrap();
        route.model_id = None;
        set_override(&mut settings, route);
        dialog.draft = settings;
        let semantic = resolve_route(&dialog.draft, ContractId::Semantic);

        // No credential: the first thing missing.
        assert_eq!(
            dialog.readiness(&semantic),
            Readiness::Blocked(NO_KEY.to_string())
        );

        // With one, the missing model is next — this is the exact state that
        // used to answer `Ready`.
        dialog.credentials = CredentialState::File {
            fingerprint: "0a1b2c3d".to_string(),
            label: None,
        };
        assert_eq!(
            dialog.readiness(&semantic),
            Readiness::Blocked(NO_MODEL.to_string())
        );

        // With a model but no catalog, "we have not looked" rather than a
        // confident claim in either direction.
        let mut settings = AssistSettings::default();
        set_override(
            &mut settings,
            recommended_route(ContractId::Semantic).unwrap(),
        );
        dialog.draft = AssistSettings::default();
        let semantic = resolve_route(&dialog.draft, ContractId::Semantic);
        assert!(matches!(dialog.readiness(&semantic), Readiness::Unknown(_)));

        // A catalog whose entries the current constraints all filter out blocks,
        // and names the constraint rather than the credential.
        dialog.catalog = CacheSlot::Loaded(audio_catalog(&["acme/never-measured"]));
        assert!(!dialog.draft.catalog.show_experimental);
        assert_eq!(
            dialog.readiness(&semantic),
            Readiness::Blocked(NO_ENDPOINT.to_string())
        );

        // And with everything present, Ready — the same catalog, once the filter
        // that emptied it is turned off.
        dialog.draft.catalog.show_experimental = true;
        assert_eq!(dialog.readiness(&semantic), Readiness::Ready);
    }

    /// S3. The eligibility count ignores the selected id, or a route pointed at
    /// a model the catalog never heard of would be its own evidence.
    #[test]
    fn the_eligible_count_does_not_count_the_selection_itself() {
        let mut dialog = dialog();
        dialog.catalog = CacheSlot::Loaded(audio_catalog(&["text/only"]));
        dialog.draft.catalog.show_experimental = true;
        // The picker keeps the selected id so it stays changeable...
        assert!(dialog
            .model_options_for(ContractId::Semantic)
            .contains(&"xiaomi/mimo-v2.5".to_string()));
        // ...and readiness still sees zero eligible entries.
        assert_eq!(dialog.openrouter_eligible(ContractId::Semantic), 1);
        dialog.catalog = CacheSlot::Loaded(CatalogCache {
            schema_version: "musializer.openrouter-catalog/v1".to_string(),
            source_url: String::new(),
            fetched_at_utc: None,
            filters: BTreeMap::new(),
            models: vec![CatalogModel {
                id: "text/only".to_string(),
                input_modalities: vec!["text".to_string()],
                output_modalities: vec!["text".to_string()],
                ..CatalogModel::default()
            }],
        });
        assert_eq!(dialog.openrouter_eligible(ContractId::Semantic), 0);
    }

    /// S4. The focus moving arms the follow, and a chrome control does not.
    #[test]
    fn moving_the_focus_arms_the_scroll_to_follow_it() {
        let mut dialog = dialog();
        dialog.section = Section::LocalModels;
        assert!(!dialog.focus_follow);
        dialog.focus_next(1);
        assert!(dialog.focus_follow, "Tab did not ask the view to follow");

        // Close and the tabs live in the chrome, which does not scroll.
        dialog.focus = 0;
        dialog.refresh_ring();
        assert_eq!(dialog.focused(), Some(FOCUS_CLOSE));
        assert!(!dialog.focus_is_in_body());
        let last = dialog.ring.len() - 1;
        dialog.focus = last;
        assert_eq!(dialog.focused(), Some(FOCUS_MODELS_DOCTOR));
        assert!(dialog.focus_is_in_body());
    }

    /// S5. The ring and the drawing code share one expression per control, so a
    /// tabstop can never land on something drawn disabled. Asserted in both
    /// directions, because "the ring is a subset" alone is satisfied by an empty
    /// ring.
    #[test]
    fn the_ring_holds_exactly_the_controls_that_are_enabled() {
        let mut dialog = dialog();
        dialog.credentials = CredentialState::File {
            fingerprint: "0a1b2c3d".to_string(),
            label: None,
        };
        dialog.draft.catalog.network_allowed = true;
        for section in Section::ALL {
            dialog.section = section;
            let ring = dialog.focus_order();
            for (index, contract) in ALL_CONTRACTS.into_iter().enumerate() {
                let index = index as ControlId;
                assert_eq!(
                    ring.contains(&(FOCUS_ROUTE_BASE + index)),
                    section == Section::Routing && dialog.route_cell_enabled(contract),
                    "{section:?} route {contract:?}"
                );
                assert_eq!(
                    ring.contains(&(FOCUS_MODEL_BASE + index)),
                    section == Section::Routing && dialog.model_cell_enabled(contract),
                    "{section:?} model {contract:?}"
                );
                assert_eq!(
                    ring.contains(&(FOCUS_FALLBACK_BASE + index)),
                    section == Section::Routing && dialog.fallback_cell_enabled(contract),
                    "{section:?} fallback {contract:?}"
                );
                assert_eq!(
                    ring.contains(&(FOCUS_PRIVACY_ZDR_BASE + index)),
                    section == Section::Privacy
                        && contract.max_boundary().rank() >= 2
                        && dialog.zdr_enabled(contract),
                    "{section:?} zdr {contract:?}"
                );
            }
            assert_eq!(
                ring.contains(&FOCUS_KEY_FORGET),
                section == Section::OpenRouter && dialog.key_forget_enabled()
            );
            assert_eq!(
                ring.contains(&FOCUS_CATALOG_REFRESH),
                section == Section::OpenRouter && dialog.catalog_refresh_enabled()
            );
            assert_eq!(ring.contains(&FOCUS_SAVE), dialog.save_enabled());
        }
        // And the enabled side really does vary, or the equalities above would
        // be comparing false with false everywhere.
        assert!(dialog.key_forget_enabled());
        assert!(dialog.catalog_refresh_enabled());
        assert!(dialog.save_enabled(), "the draft edit above left it clean");
        assert!(!dialog.model_cell_enabled(ContractId::Align));
    }

    /// S6. `TC-VERIFY` has no recommended route, so there is nothing for a
    /// fallback policy to apply to. It was drawn enabled and silently did
    /// nothing when pressed.
    #[test]
    fn the_fallback_picker_needs_a_route_before_it_is_offered() {
        let mut dialog = dialog();
        assert!(resolve_route(&dialog.draft, ContractId::Verify)
            .route
            .is_none());
        assert!(ContractId::Verify.allowed_fallbacks().len() > 1);
        assert!(!dialog.fallback_cell_enabled(ContractId::Verify));
        assert!(dialog.fallback_cell_enabled(ContractId::Coarse));

        // Give it a route and the picker becomes real.
        let route = Route {
            contract: ContractId::Verify,
            route_type: RouteType::LocalProc,
            runtime_id: "whisper.cpp".to_string(),
            model_id: Some("whisper.cpp".to_string()),
            model_path: None,
            reasoning_effort: None,
            fallback: FallbackPolicy::None,
            provider: None,
        };
        set_override(&mut dialog.draft, route);
        assert!(dialog.fallback_cell_enabled(ContractId::Verify));
    }

    /// S7. Catalog strings are display data. The three cases are the reviewer's
    /// own fixture: a 1322-character id, an embedded newline and tab, and a
    /// right-to-left override that a control-character strip does not catch.
    #[test]
    fn a_hostile_catalog_string_cannot_rearrange_the_page() {
        let long_id = format!("a/{}", "long-model-id-segment-".repeat(60));
        assert_eq!(long_id.chars().count(), 1322);
        let sanitized = sanitize_display(&long_id);
        assert_eq!(sanitized.chars().count(), DISPLAY_CAP + 1);
        assert!(sanitized.ends_with('\u{2026}'));

        assert_eq!(sanitize_display("c/newline\nand\ttab"), "c/newline and tab");
        assert_eq!(sanitize_display("line1\nline2"), "line1 line2");
        assert_eq!(
            sanitize_display("b/rtl-\u{202e}reversed\u{202c}-and--nul"),
            "b/rtl-reversed-and--nul"
        );
        for character in sanitize_display("a\u{0}b\u{7}c\u{200f}d\u{2066}e").chars() {
            assert!(!character.is_control() && !is_bidi_control(character));
        }
        // Whitespace collapses rather than accumulating, and nothing is padded.
        assert_eq!(
            sanitize_display("  emoji \u{1f3b5}   x  "),
            "emoji \u{1f3b5} x"
        );
        assert_eq!(sanitize_display(""), "");
        assert_eq!(sanitize_display("   "), "");
        // A string already fit for display is returned unchanged, so ordinary
        // ids do not gain an ellipsis.
        assert_eq!(sanitize_display("xiaomi/mimo-v2.5"), "xiaomi/mimo-v2.5");
    }

    /// S8. The coarse lane the production path already runs is recommended on
    /// the four-track benchmark's own Whisper evidence, so its picker is not
    /// hidden behind `Show experimental` on a fresh install.
    #[test]
    fn the_coarse_lane_is_recommended_on_the_benchmarks_evidence() {
        assert_eq!(
            suitability::status("whisper.cpp", ContractId::Coarse),
            Suitability::Recommended
        );
        let row = suitability::row("whisper.cpp", ContractId::Coarse).unwrap();
        assert_eq!(row.evidence_date, Some("2026-08-04"));
        assert_eq!(row.prompt_version, Some("anchor-block-mms/1"));
        assert_eq!(row.schema_version, Some("musializer.lyric-timing/v1"));
        assert!(row.evidence.contains("LYRICS_TIMING_BENCHMARK_RESULTS.md"));
        // Offered without asking, which is what "recommended" buys.
        let offered = model_options(
            ContractId::Coarse,
            RouteType::LocalProc,
            None,
            None,
            false,
            None,
        );
        assert_eq!(offered, vec!["whisper.cpp".to_string()]);
        // And the experimental one stays experimental.
        assert_eq!(
            suitability::status("xiaomi/mimo-v2.5", ContractId::Semantic),
            Suitability::Experimental
        );
    }

    /// S8, rewritten for defect B. With the filter on, `TC-SEMANTIC` has
    /// **nothing** to offer — not "only the selected id" — and the row still
    /// shows what the settings configure.
    #[test]
    fn a_single_option_picker_knows_whether_the_filter_emptied_it() {
        let mut dialog = dialog();
        dialog.catalog =
            CacheSlot::Loaded(audio_catalog(&["xiaomi/mimo-v2.5", "acme/never-measured"]));
        // Experimental off: both catalog models are unmeasured, so the picker
        // offers nothing at all. This is the operator's screenshot.
        assert!(dialog.model_options_for(ContractId::Semantic).is_empty());
        assert!(!dialog.model_cell_enabled(ContractId::Semantic));
        assert_eq!(
            dialog
                .model_options_with_experimental(ContractId::Semantic)
                .len(),
            2
        );
        // The configuration is still readable, and the row says in words that it
        // is a configuration rather than an offer.
        assert_eq!(
            dialog.configured_model(ContractId::Semantic).as_deref(),
            Some("xiaomi/mimo-v2.5")
        );
        assert_eq!(
            dialog.configured_status(ContractId::Semantic),
            Some(Suitability::Experimental)
        );
        assert_eq!(
            dialog.contracts_without_an_offer(),
            vec![ContractId::Semantic],
            "the contract with no offerable model is not named"
        );
        dialog.draft.catalog.show_experimental = true;
        assert!(dialog.model_cell_enabled(ContractId::Semantic));
        assert!(dialog.contracts_without_an_offer().is_empty());
    }

    /// Defect B. One offered model plus a configured one that is filtered out is
    /// the case `len() > 1` drew disabled — the single press that moves the
    /// contract onto something that *is* offered.
    #[test]
    fn a_picker_is_enabled_when_its_one_option_is_not_what_is_configured() {
        let mut dialog = dialog();
        // `mms-ctc` is the recommended aligner and `qwen3-fa` is unsupported, so
        // TC-ALIGN offers exactly one model at either toggle setting.
        let mut route = recommended_route(ContractId::Align).unwrap();
        assert_eq!(dialog.model_options_for(ContractId::Align).len(), 1);
        assert!(
            !dialog.model_cell_enabled(ContractId::Align),
            "the one option is what is configured, so there is nowhere to go"
        );
        // Point it at a model nothing offers. The one offered option is now
        // somewhere else, and the picker has to be able to reach it.
        route.model_id = Some("qwen3-fa".to_string());
        set_override(&mut dialog.draft, route);
        assert_eq!(dialog.model_options_for(ContractId::Align).len(), 1);
        assert!(
            dialog.model_cell_enabled(ContractId::Align),
            "a picker with one reachable option was drawn disabled"
        );
        assert_eq!(
            dialog.configured_status(ContractId::Align),
            Some(Suitability::Unsupported)
        );
    }

    /// S9. The plain-language names are drawn, and a badge's tooltip carries the
    /// whole remediation the 96 px pill ellipsized away.
    #[test]
    fn a_badge_hint_carries_what_the_pill_cannot() {
        assert_eq!(
            ContractId::Coarse.human_label(),
            "Coarse lyric localization"
        );
        assert_eq!(
            ContractId::Verify.human_label(),
            "Independent timing verification"
        );
        let key = Readiness::Blocked(NO_KEY.to_string());
        assert!(key.hint().contains("OpenRouter section"));
        assert!(key.hint().len() > key.label().len() * 4);
        assert!(Readiness::Blocked(NO_MODEL.to_string())
            .hint()
            .contains("MODEL column"));
        assert!(Readiness::Blocked(NO_ENDPOINT.to_string())
            .hint()
            .contains("Show experimental"));
        assert!(Readiness::Unknown("Run doctor".to_string())
            .hint()
            .starts_with("Run doctor"));
        assert!(Readiness::Ready.hint().contains("credential"));
    }

    /// S11. The dialog can say whether the promise it makes in its subtitle is
    /// about a job that exists.
    #[test]
    fn a_running_job_is_named_in_the_report() {
        let mut dialog = dialog();
        assert!(dialog.describe()[0].contains("job-running=false"));
        dialog.set_job_running(true);
        assert!(dialog.describe()[0].contains("job-running=true"));
    }

    /// S12. Three faults, three fixes, and the schema one names the entry. The
    /// second pass reads key names only — the canary is in the file's `secret`
    /// values and must reach neither the state nor the report.
    #[test]
    fn a_credentials_fault_is_diagnosed_rather_than_called_invalid_json() {
        use musializer_core::assist::credentials::CredentialsError;
        let directory = std::env::temp_dir().join(format!(
            "musializer-ap3r-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let not_json = directory.join("not-json.json");
        std::fs::write(&not_json, b"{ broken").unwrap();
        let state = classify_credential_fault(
            &not_json,
            &AssistFileError::Credentials(CredentialsError::Format),
        );
        assert!(matches!(
            state,
            CredentialState::Unusable {
                kind: CredentialFault::NotJson,
                ..
            }
        ));
        assert_eq!(state.token(), "unusable(not-json)");

        // Valid JSON, wrong entry shape — the reviewer's `corruptentry` fixture.
        let bad_entry = directory.join("bad-entry.json");
        std::fs::write(
            &bad_entry,
            br#"{"schema":"musializer.assist-credentials/v1","entries":{
                "openrouter/default":{"secret":12345},
                "someoneelse/work":{"secret":"sk-other-KEEPME99"}}}"#,
        )
        .unwrap();
        let state = classify_credential_fault(
            &bad_entry,
            &AssistFileError::Credentials(CredentialsError::Format),
        );
        match &state {
            CredentialState::Unusable { kind, entry, .. } => {
                assert_eq!(*kind, CredentialFault::Schema);
                assert_eq!(entry.as_deref(), Some("openrouter/default"));
            }
            other => panic!("expected a schema fault, got {other:?}"),
        }
        assert!(state.token().contains("entry=openrouter/default"));
        // The other entry's secret is in the same file and must not travel.
        assert!(!format!("{state:?}").contains("KEEPME99"));
        assert!(!CredentialFault::Schema.remediation().contains("KEEPME99"));

        let state = classify_credential_fault(
            &directory.join("absent.json"),
            &AssistFileError::Read(directory.join("absent.json")),
        );
        assert!(matches!(
            state,
            CredentialState::Unusable {
                kind: CredentialFault::Io,
                ..
            }
        ));
        // Each fault names a different thing to do.
        let fixes: std::collections::BTreeSet<&str> = [
            CredentialFault::Io,
            CredentialFault::NotJson,
            CredentialFault::Schema,
            CredentialFault::TooLarge,
        ]
        .into_iter()
        .map(CredentialFault::remediation)
        .collect();
        assert_eq!(fixes.len(), 4);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// S13. The reviewer's suspected desync, reproduced and made unstateable.
    ///
    /// `forget_key` used to put the cleared credential record only in the
    /// *draft*, so Escape-discard threw it away and `saved` — what the rest of
    /// the dialog reads, and what the next Save writes — still claimed
    /// `mode: file` with the fingerprint of a key that no longer existed. A
    /// credential operation already happened on disk; it is not an edit.
    #[test]
    fn a_credential_operation_survives_an_escape_discard() {
        let mut dialog = dialog();
        let stored = musializer_core::assist::settings::CredentialRecord {
            mode: CredentialMode::File,
            lookup_id: "default".to_string(),
            fingerprint: Some("0a1b2c3d".to_string()),
            label: Some("Personal".to_string()),
        };
        dialog.saved.credentials.openrouter = stored.clone();
        dialog.draft.credentials.openrouter = stored;
        assert!(!dialog.is_dirty());

        // No settings path, so nothing is written to disk — the in-memory
        // agreement is the property under test.
        dialog.settings_path = None;
        let persisted = dialog
            .record_credential(musializer_core::assist::settings::CredentialRecord::default());
        assert!(!persisted);

        // Both stores moved, so the operation did not arm the dirty flag...
        assert!(
            !dialog.is_dirty(),
            "a credential operation was staged as an unsaved edit"
        );
        // ...and a discard cannot bring the forgotten key back.
        dialog.draft = dialog.saved.clone();
        assert_eq!(
            dialog.saved.credentials.openrouter,
            musializer_core::assist::settings::CredentialRecord::default()
        );
        assert_eq!(
            dialog.saved.credentials.openrouter.mode,
            CredentialMode::None
        );
        assert!(dialog.saved.credentials.openrouter.fingerprint.is_none());
        assert!(dialog.draft.credentials.openrouter.fingerprint.is_none());

        // And an operation made *while* there are other unsaved edits leaves the
        // file alone rather than sweeping them in.
        dialog.draft.catalog.show_experimental = true;
        assert!(dialog.is_dirty());
        let persisted =
            dialog.record_credential(musializer_core::assist::settings::CredentialRecord {
                mode: CredentialMode::File,
                lookup_id: "default".to_string(),
                fingerprint: Some("beefcafe".to_string()),
                label: None,
            });
        assert!(!persisted);
        assert_eq!(
            dialog.saved.credentials.openrouter.fingerprint.as_deref(),
            Some("beefcafe")
        );
        assert!(dialog.is_dirty(), "the unrelated edit was swallowed");
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
