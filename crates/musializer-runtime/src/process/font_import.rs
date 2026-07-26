//! Font import supervision and its bounded catalogue reader.
//!
//! **Owner: Agent E.** Ported from the frozen C oracle at `../musializer`
//! (commit `9300af9`, read-only): `src/font_catalogue.c/.h` for the readers, and
//! `find_font_helper` / `font_job_start` / `poll_font_job` / `font_job_cancel`
//! (`src/plug.c:1551-1861`) for the supervision.
//!
//! Two halves, and the boundary between them is the whole design:
//!
//! 1. **The readers.** `tools/google_fonts.py` is an independent
//!    evidence-producing tool. It talks to the network; this does not. The
//!    entire interface is two bounded TSV files it writes — a family list and a
//!    one-row download manifest — and everything in them is a *claim* that gets
//!    validated before it can reach a project. There is no JSON parser here on
//!    purpose (`src/font_catalogue.h:8-11`).
//! 2. **The job.** A supervised child, the third family alongside FFmpeg export
//!    and Assist analysis. Every job writes into its own nonce-named directory
//!    so a cancelled worker that keeps writing writes somewhere nothing will
//!    ever read (`src/plug.c:1629-1631`).
//!
//! What this module deliberately does **not** do: re-hash the downloaded files,
//! rasterize the face, or touch a project. The manifest's digests are claims
//! and the caller re-verifies them (`src/plug.c:1750-1761`) using Agent B's
//! `sha256`; the face becomes the caption face in the same step that records the
//! asset, which is Agent B's and Agent F's business, not this one's.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::process_group::{
    kill_group_and_reap, signal_process, wait_bounded, ShutdownPolicy, Signal, WaitOutcome,
};

// ---------------------------------------------------------------------------
// Bounds. Every one of these is a ceiling on something another process wrote.
// ---------------------------------------------------------------------------

/// `FONT_CATALOGUE_SCHEMA_HEADER` (`src/font_catalogue.h:17`).
pub const CATALOGUE_SCHEMA_HEADER: &str = "musializer.font-catalogue/v1";
/// `FONT_CATALOGUE_MAX_FAMILIES` (`:21`). Google Fonts offers roughly 1800 Latin
/// families; the cap absorbs growth while keeping a catalogue a quarter of a
/// megabyte rather than an unbounded read.
pub const CATALOGUE_MAX_FAMILIES: usize = 4096;
/// `FONT_CATALOGUE_FAMILY_CAPACITY` (`:25`), as a **maximum length in bytes**.
/// The C constant is a buffer size including the NUL, so the longest usable name
/// is one less — which is what this is.
pub const FAMILY_MAX_BYTES: usize = 63;
/// `FONT_CATALOGUE_MAX_BYTES` (`:26`).
pub const CATALOGUE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// `FONT_IMPORT_MANIFEST_HEADER` (`:100`).
pub const MANIFEST_SCHEMA_HEADER: &str = "musializer.font-import/v1";
/// `FONT_IMPORT_MANIFEST_INDEX_NAME` (`:101`).
pub const MANIFEST_INDEX_NAME: &str = "import.tsv";
/// `FONT_IMPORT_PATH_CAPACITY` (`:102`), again as a maximum length.
pub const IMPORT_PATH_MAX_BYTES: usize = 1023;
/// `FONT_IMPORT_LICENCE_NAME_CAPACITY` (`:104`), as a maximum length.
pub const LICENCE_NAME_MAX_BYTES: usize = 63;

/// `Font_Category` (`src/font_catalogue.h:28-36`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontCategory {
    /// A category this build has never heard of. Not an error: the catalogue is
    /// allowed to grow one, and a family that only sorts oddly is still usable
    /// (`src/font_catalogue.c:90-92`).
    #[default]
    Unknown,
    SansSerif,
    Serif,
    Display,
    Handwriting,
    Monospace,
}

impl FontCategory {
    /// `category_from_name` (`src/font_catalogue.c:75-93`). Matched on the exact
    /// spellings the helper writes.
    fn from_field(field: &str) -> Self {
        match field {
            "Sans Serif" => FontCategory::SansSerif,
            "Serif" => FontCategory::Serif,
            "Display" => FontCategory::Display,
            "Handwriting" => FontCategory::Handwriting,
            "Monospace" => FontCategory::Monospace,
            _ => FontCategory::Unknown,
        }
    }

    /// `font_category_name` (`:22-35`). An unknown category displays as
    /// "Other", not as "Unknown".
    pub const fn display_name(self) -> &'static str {
        match self {
            FontCategory::SansSerif => "Sans Serif",
            FontCategory::Serif => "Serif",
            FontCategory::Display => "Display",
            FontCategory::Handwriting => "Handwriting",
            FontCategory::Monospace => "Monospace",
            FontCategory::Unknown => "Other",
        }
    }
}

/// Script coverage bits, `FONT_SCRIPT_*` (`src/font_catalogue.h:41-45`).
///
/// Only the scripts the caption glyph set actually covers. A family may well
/// carry Devanagari; this build's atlas would not rasterize it, so recording it
/// would be a promise the renderer cannot keep.
pub mod script {
    pub const LATIN: u32 = 1 << 0;
    pub const LATIN_EXT: u32 = 1 << 1;
    pub const GREEK: u32 = 1 << 2;
    pub const CYRILLIC: u32 = 1 << 3;
    pub const VIETNAMESE: u32 = 1 << 4;
}

#[rustfmt::skip]
const SCRIPT_FIELDS: [(&str, u32); 5] = [
    ("latin",      script::LATIN),
    ("latin-ext",  script::LATIN_EXT),
    ("greek",      script::GREEK),
    ("cyrillic",   script::CYRILLIC),
    ("vietnamese", script::VIETNAMESE),
];

#[rustfmt::skip]
const SCRIPT_NAMES: [(u32, &str); 5] = [
    (script::LATIN,      "Latin"),
    (script::LATIN_EXT,  "Latin Ext"),
    (script::GREEK,      "Greek"),
    (script::CYRILLIC,   "Cyrillic"),
    (script::VIETNAMESE, "Vietnamese"),
];

/// `scripts_from_list` (`src/font_catalogue.c:95-119`). Unknown script names are
/// skipped, not rejected.
fn scripts_from_field(field: &str) -> u32 {
    let mut scripts = 0;
    for item in field.split(',') {
        if let Some((_, bit)) = SCRIPT_FIELDS.iter().find(|(name, _)| *name == item) {
            scripts |= bit;
        }
    }
    scripts
}

/// Human-readable script coverage, e.g. `"Latin, Greek"`.
///
/// `font_scripts_describe` (`src/font_catalogue.c:37-67`) without the C's
/// truncation, which only existed because the caller passed a fixed buffer. Use
/// [`describe_scripts_bounded`] where the C's exact truncation matters.
pub fn describe_scripts(scripts: u32) -> String {
    let names: Vec<&str> = SCRIPT_NAMES
        .iter()
        .filter(|(bit, _)| scripts & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if names.is_empty() {
        return "no known script".to_string();
    }
    names.join(", ")
}

/// [`describe_scripts`] truncated to `capacity` bytes **including** the C's NUL,
/// so the C's buffer semantics are reproducible where a caller cares.
///
/// The rule worth keeping (`src/font_catalogue.c:57-62`): a list that runs out
/// of room stops after the last **whole** name, because "Latin, Cyril" reads as
/// a script nobody has heard of. `capacity == 0` produces nothing, and
/// `capacity` too small even for the first name produces an empty string rather
/// than the phrase that means the face covers nothing.
pub fn describe_scripts_bounded(scripts: u32, capacity: usize) -> String {
    if capacity == 0 {
        return String::new();
    }
    let mut output = String::new();
    let mut any = false;
    for (bit, name) in SCRIPT_NAMES {
        if scripts & bit == 0 {
            continue;
        }
        any = true;
        let separator = if output.is_empty() { "" } else { ", " };
        let addition = separator.len() + name.len();
        // snprintf writes at most `capacity - 1 - written` bytes plus a NUL, so
        // the addition fits only if it is strictly shorter than the room left.
        if addition >= capacity - output.len() {
            return output;
        }
        output.push_str(separator);
        output.push_str(name);
    }
    if !any {
        let phrase = "no known script";
        return phrase[..phrase.len().min(capacity - 1)].to_string();
    }
    output
}

/// One catalogue row, `Font_Catalogue_Entry` (`src/font_catalogue.h:47-51`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub family: String,
    pub category: FontCategory,
    pub scripts: u32,
}

/// The parsed family list, `Font_Catalogue` (`:53-56`).
#[derive(Debug, Clone, Default)]
pub struct FontCatalogue {
    entries: Vec<CatalogueEntry>,
}

/// `Font_Catalogue_Result` (`src/font_catalogue.h:58-67`), with the messages from
/// `font_catalogue_result_string` (`src/font_catalogue.c:6-20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueError {
    #[error("catalogue is larger than this build will read")]
    TooLarge,
    #[error("catalogue header is missing or from another version")]
    Header,
    #[error("catalogue row is malformed")]
    Row,
    #[error("catalogue family name is empty, too long, or contains control characters")]
    Family,
    #[error("catalogue holds more families than this build can list")]
    Capacity,
    #[error("catalogue contains no families")]
    Empty,
}

/// A family name is a label drawn into the interface **and** a token handed back
/// to the helper as a command-line argument. Control characters in either place
/// are a problem, so they are rejected on the way in rather than escaped later
/// (`src/font_catalogue.c:121-132`).
///
/// The length limit counts bytes, matching the C's fixed buffer.
fn family_is_printable(family: &str) -> bool {
    if family.is_empty() || family.len() > FAMILY_MAX_BYTES {
        return false;
    }
    !family.bytes().any(|byte| byte < 0x20 || byte == 0x7f)
}

/// `split_row` (`src/font_catalogue.c:134-148`): exactly `expected` tab-separated
/// fields, no more and no fewer. Ragged rows are rejected rather than padded.
fn split_row(row: &str, expected: usize) -> Option<Vec<&str>> {
    let fields: Vec<&str> = row.split('\t').collect();
    if fields.len() == expected {
        Some(fields)
    } else {
        None
    }
}

/// Splits off the header line and checks its schema token.
///
/// The C compares only the **first** field against the schema string and ignores
/// the second (a row count), while still requiring exactly two fields
/// (`src/font_catalogue.c:159-169`). Both parts are reproduced: a header with a
/// stale count still parses, a header with one or three fields does not.
fn take_header<'a>(text: &'a str, schema: &str) -> Result<&'a str, CatalogueError> {
    let (header, rest) = text.split_once('\n').ok_or(CatalogueError::Header)?;
    let fields = split_row(header, 2).ok_or(CatalogueError::Header)?;
    if fields[0] != schema {
        return Err(CatalogueError::Header);
    }
    Ok(rest)
}

impl FontCatalogue {
    /// The families, in the catalogue's own order.
    ///
    /// That order is popularity order and must be preserved: re-sorting it
    /// alphabetically would bury every face anyone wants
    /// (`tests/test_font_catalogue.c:34-35`).
    pub fn entries(&self) -> &[CatalogueEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `font_catalogue_clear` (`src/font_catalogue.c:69-73`).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Parses a catalogue, **replacing `self` only on success**.
    ///
    /// `font_catalogue_parse` (`src/font_catalogue.c:150-208`). A failed refresh
    /// must not empty a working picker, which is what
    /// `tests/test_font_catalogue.c:85-88` pins.
    ///
    /// **Divergence, deliberate.** The C claims to parse into a scratch count
    /// but writes each row straight into `destination->entries` and only
    /// withholds `count` on failure (`:196-206`). So a failure part-way through
    /// leaves the previous catalogue's *count* with the new catalogue's *rows* —
    /// a silently corrupted picker. This builds a local `Vec` and commits it, so
    /// the header's documented contract holds. Reproducing the clobber would be
    /// reproducing a bug the tests do not cover; it is noted rather than copied.
    pub fn parse_replace(&mut self, text: &str) -> Result<(), CatalogueError> {
        let parsed = Self::parse(text)?;
        *self = parsed;
        Ok(())
    }

    /// Parses a catalogue into a fresh value.
    pub fn parse(text: &str) -> Result<Self, CatalogueError> {
        if text.len() > CATALOGUE_MAX_BYTES {
            return Err(CatalogueError::TooLarge);
        }
        let body = take_header(text, CATALOGUE_SCHEMA_HEADER)?;

        let mut entries = Vec::new();
        for row in body.split('\n') {
            // A trailing newline leaves an empty final row, and blank rows in
            // the middle are skipped rather than counted (`:179-184`).
            if row.is_empty() {
                continue;
            }
            let fields = split_row(row, 3).ok_or(CatalogueError::Row)?;
            if !family_is_printable(fields[0]) {
                return Err(CatalogueError::Family);
            }
            if entries.len() >= CATALOGUE_MAX_FAMILIES {
                return Err(CatalogueError::Capacity);
            }
            entries.push(CatalogueEntry {
                family: fields[0].to_string(),
                category: FontCategory::from_field(fields[1]),
                scripts: scripts_from_field(fields[2]),
            });
        }
        if entries.is_empty() {
            return Err(CatalogueError::Empty);
        }
        Ok(FontCatalogue { entries })
    }

    /// Case-insensitive substring match on the family name, preserving the
    /// catalogue's order and filtering by required script coverage.
    ///
    /// `font_catalogue_filter` (`src/font_catalogue.c:239-256`). Returns the
    /// **total** number that matched even when the caller wanted fewer indices,
    /// which is what lets a picker say "showing 1 of 3" instead of silently
    /// truncating (`tests/test_font_catalogue.c:164-165`).
    pub fn filter(&self, query: &str, required_scripts: u32, indices: &mut [usize]) -> FilterCount {
        let mut matched = 0;
        let mut written = 0;
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.scripts & required_scripts != required_scripts {
                continue;
            }
            if !entry.matches(query) {
                continue;
            }
            matched += 1;
            if written < indices.len() {
                indices[written] = index;
                written += 1;
            }
        }
        FilterCount { matched, written }
    }

    /// Exact family lookup, which is what turns a saved family name back into a
    /// catalogue row.
    ///
    /// `font_catalogue_find` (`src/font_catalogue.c:331-342`). Exact, not folded:
    /// resolving `"space mono"` or `"Space"` to a different face would silently
    /// retypeset someone's captions (`tests/test_font_catalogue.c:181-184`).
    pub fn find(&self, family: &str) -> Option<usize> {
        if family.is_empty() {
            return None;
        }
        self.entries.iter().position(|entry| entry.family == family)
    }
}

/// How many rows matched a filter, and how many fitted in the caller's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterCount {
    pub matched: usize,
    pub written: usize,
}

impl CatalogueEntry {
    /// `font_catalogue_entry_matches` (`src/font_catalogue.c:232-237`), which
    /// folds case ASCII-only. That is all the family names contain: the helper
    /// validates them against an ASCII pattern before they reach the index
    /// (`:216-217`).
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let haystack = self.family.as_bytes();
        let needle = query.as_bytes();
        if needle.len() > haystack.len() {
            return false;
        }
        haystack.windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
        })
    }
}

/// One completed download, as the helper described it.
///
/// `Font_Import_Manifest` (`src/font_catalogue.h:108-115`). Every path and digest
/// here is a **claim**: the caller re-hashes both files before either is used
/// (`src/plug.c:1750-1761`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportManifest {
    pub family: String,
    pub font_path: String,
    pub font_sha256: String,
    pub licence_path: String,
    pub licence_sha256: String,
    pub licence_name: String,
}

/// Lowercase hex SHA-256, `sha256_hex` (`src/font_catalogue.c:258-267`).
///
/// Uppercase is rejected on purpose: accepting it would mean comparing it
/// against a real hash and always failing, with the confusing message that the
/// download did not match (`tests/test_font_catalogue.c:217-219`).
fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `copy_field` (`src/font_catalogue.c:269-276`): non-empty and short enough.
fn bounded_field(text: &str, max_bytes: usize) -> Option<String> {
    if text.is_empty() || text.len() > max_bytes {
        return None;
    }
    Some(text.to_string())
}

impl ImportManifest {
    /// Reads the single manifest row.
    ///
    /// `font_import_manifest_parse` (`src/font_catalogue.c:278-329`). Rejects a
    /// missing header, the wrong column count, an over-long field, a malformed
    /// digest, and an empty family, path or licence name.
    pub fn parse(text: &str) -> Result<Self, CatalogueError> {
        if text.len() > CATALOGUE_MAX_BYTES {
            return Err(CatalogueError::TooLarge);
        }
        let body = take_header(text, MANIFEST_SCHEMA_HEADER)?;
        if body.is_empty() {
            return Err(CatalogueError::Empty);
        }
        let row = body.split('\n').next().unwrap_or_default();
        let fields = split_row(row, 6).ok_or(CatalogueError::Row)?;
        if !family_is_printable(fields[0]) {
            return Err(CatalogueError::Family);
        }
        // Built into a local first, so a manifest that fails on its last field
        // cannot leave the caller holding half of a previous import.
        let parsed = ImportManifest {
            family: bounded_field(fields[0], FAMILY_MAX_BYTES).ok_or(CatalogueError::Row)?,
            font_path: bounded_field(fields[1], IMPORT_PATH_MAX_BYTES)
                .ok_or(CatalogueError::Row)?,
            font_sha256: if is_sha256_hex(fields[2]) {
                fields[2].to_string()
            } else {
                return Err(CatalogueError::Row);
            },
            licence_path: bounded_field(fields[3], IMPORT_PATH_MAX_BYTES)
                .ok_or(CatalogueError::Row)?,
            licence_sha256: if is_sha256_hex(fields[4]) {
                fields[4].to_string()
            } else {
                return Err(CatalogueError::Row);
            },
            licence_name: bounded_field(fields[5], LICENCE_NAME_MAX_BYTES)
                .ok_or(CatalogueError::Row)?,
        };
        Ok(parsed)
    }
}

// ---------------------------------------------------------------------------
// The supervised child.
// ---------------------------------------------------------------------------

/// Which helper subcommand a job runs, `Font_Import_Job`
/// (`src/font_import_state.h:26-30`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontJobKind {
    /// `google_fonts.py catalogue <cache> --index <artifact>`: one request
    /// against a static file.
    Catalogue,
    /// `google_fonts.py fetch <family> <directory>`: three requests and a few
    /// hundred kilobytes.
    Fetch,
}

impl FontJobKind {
    /// `font_import_job_timeout` (`src/font_import_state.c:14-22`), from
    /// `FONT_CATALOGUE_JOB_TIMEOUT_SECONDS` and
    /// `FONT_FETCH_JOB_TIMEOUT_SECONDS` (`src/font_import_state.h:20-21`).
    pub const fn timeout(self) -> Duration {
        match self {
            FontJobKind::Catalogue => Duration::from_secs(120),
            FontJobKind::Fetch => Duration::from_secs(180),
        }
    }
}

/// `font_import_next_nonce` (`src/font_import_state.c:122-126`). Never zero,
/// because zero is the "no job" value.
pub const fn next_nonce(nonce: u64) -> u64 {
    match nonce.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

/// A job's result is only the result the user is waiting for when its nonce is
/// the one that was issued last.
///
/// `font_import_result_is_current` (`src/font_import_state.h:89-93`). Anything
/// else is a job that was cancelled or superseded while it was still writing,
/// and applying it would replace the caption face with one nobody asked for.
pub const fn result_is_current(result_nonce: u64, active_nonce: u64) -> bool {
    result_nonce != 0 && result_nonce == active_nonce
}

/// Why a font job could not be started or read.
#[derive(Debug, thiserror::Error)]
pub enum FontJobError {
    /// `"The font helper is missing from this installation."`
    /// (`src/plug.c:1622`).
    #[error("the font helper is missing from this installation")]
    HelperMissing,
    /// `"The font workspace could not be created. …"` (`:1626`), and
    /// `"A unique font workspace could not be reserved."` (`:1638`).
    #[error("the font workspace could not be created: {0}")]
    Workspace(#[source] io::Error),
    #[error("a fetch job needs a family name")]
    NoFamily,
    /// `"Python could not launch the font helper. …"` (`:1689`).
    #[error("python could not launch the font helper: {0}")]
    Spawn(#[source] io::Error),
    /// The helper finished without writing a readable artifact
    /// (`font_job_read_artifact`, `:1594-1615`): missing, empty, or over the
    /// 4 MiB ceiling. A worker killed mid-write lands here, which is the point.
    #[error("the helper finished without writing a usable result")]
    UnreadableArtifact,
    #[error("{0}")]
    Catalogue(#[from] CatalogueError),
}

/// Resolves `tools/google_fonts.py`.
///
/// `find_font_helper` (`src/plug.c:1551-1570`). Two things must be preserved:
///
/// - **The candidate list**, because the layout differs between an extracted
///   distribution (`<appdir>/tools/…`) and a source run out of `./build`
///   (`<appdir>/../tools/…`), with `./tools/…` as the final fallback.
/// - **`MUSIALIZER_FONT_HELPER` fails hard.** A set-but-missing override
///   returns `None`; it does **not** fall back to probing (`:1554-1558`). The
///   same is true of `MUSIALIZER_ASSIST_HELPER`. Somebody who points the
///   override at a typo should be told the helper is missing, not silently given
///   a different one.
///
/// `application_directory` is raylib's `GetApplicationDirectory()` — the
/// directory of the running executable, **not** the working directory.
pub fn find_font_helper(application_directory: &Path) -> Option<PathBuf> {
    find_helper(
        application_directory,
        "MUSIALIZER_FONT_HELPER",
        "google_fonts.py",
    )
}

/// The same resolution for `tools/external_analysis.py`
/// (`find_assist_helper`, `src/plug.c:2048-2067`). Shared because the two
/// functions are identical in the C apart from the variable and the filename,
/// and two copies would drift.
pub fn find_assist_helper(application_directory: &Path) -> Option<PathBuf> {
    find_helper(
        application_directory,
        "MUSIALIZER_ASSIST_HELPER",
        "external_analysis.py",
    )
}

fn find_helper(
    application_directory: &Path,
    override_variable: &str,
    file_name: &str,
) -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os(override_variable) {
        if !override_path.is_empty() {
            let path = PathBuf::from(override_path);
            // Hard failure, not a fallback. See the doc comment.
            return path.is_file().then_some(path);
        }
    }
    let candidates = [
        application_directory.join("tools").join(file_name),
        application_directory
            .join("..")
            .join("tools")
            .join(file_name),
        PathBuf::from("./tools").join(file_name),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

/// What a poll found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontJobPoll {
    /// Still working.
    Running,
    /// The helper exited zero. The artifact is ready to read.
    Succeeded,
    /// The helper exited non-zero or died from a signal. The job log has the
    /// reason (`src/plug.c:1851-1857`).
    Failed,
    /// A cancellation this supervisor requested has completed (`:1844-1849`).
    Cancelled,
    /// The job outran [`FontJobKind::timeout`] and was killed (`:1829-1839`).
    TimedOut,
}

/// A running `google_fonts.py` child and the workspace it writes into.
///
/// Every job gets its own directory named for the nonce it will be recognised
/// by, so a cancelled worker that keeps writing writes somewhere nothing will
/// ever read (`src/plug.c:1629-1631`). That is why cancellation here does not
/// need to be synchronous to be safe.
#[derive(Debug)]
pub struct FontJob {
    child: Option<Child>,
    kind: FontJobKind,
    nonce: u64,
    family: String,
    directory: PathBuf,
    artifact: PathBuf,
    log: PathBuf,
    started: std::time::Instant,
    cancelling: bool,
}

impl FontJob {
    /// Spawns the helper.
    ///
    /// `font_job_start` (`src/plug.c:1617-1700`). `workspace_root` is
    /// `./build/fonts` in the C; `previous_nonce` is the last issued nonce, so
    /// the job's own nonce is [`next_nonce`] of it.
    pub fn start(
        helper: &Path,
        workspace_root: &Path,
        kind: FontJobKind,
        family: &str,
        previous_nonce: u64,
    ) -> Result<Self, FontJobError> {
        if kind == FontJobKind::Fetch && family.is_empty() {
            return Err(FontJobError::NoFamily);
        }
        if !helper.is_file() {
            return Err(FontJobError::HelperMissing);
        }
        std::fs::create_dir_all(workspace_root).map_err(FontJobError::Workspace)?;

        let nonce = next_nonce(previous_nonce);
        let directory = workspace_root.join(format!("{nonce:016x}"));
        std::fs::create_dir_all(&directory).map_err(FontJobError::Workspace)?;
        let log = directory.join("job.log");
        let artifact = match kind {
            FontJobKind::Catalogue => directory.join("catalogue.tsv"),
            FontJobKind::Fetch => directory.join(MANIFEST_INDEX_NAME),
        };

        let mut command = Command::new("python3");
        command.arg(helper);
        match kind {
            FontJobKind::Catalogue => {
                // The reduced JSON cache is shared across jobs so a second
                // browse in the same week does not fetch three megabytes again;
                // only the index is per-job (`src/plug.c:1676-1680`).
                command
                    .arg("catalogue")
                    .arg(workspace_root.join("catalogue.json"))
                    .arg("--index")
                    .arg(&artifact);
            }
            FontJobKind::Fetch => {
                command.arg("fetch").arg(family).arg(&directory);
            }
        }
        let log_file = std::fs::File::create(&log).map_err(FontJobError::Workspace)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // stderr to the job log, as nob's `.stderr_path` does
            // (`thirdparty/nob.h:1065-1068`, O_WRONLY|O_CREAT|O_TRUNC).
            .stderr(Stdio::from(log_file));

        let child = command.spawn().map_err(FontJobError::Spawn)?;
        Ok(FontJob {
            child: Some(child),
            kind,
            nonce,
            family: family.to_string(),
            directory,
            artifact,
            log,
            started: std::time::Instant::now(),
            cancelling: false,
        })
    }

    pub fn kind(&self) -> FontJobKind {
        self.kind
    }

    /// The nonce this job will be recognised by. Compare it with
    /// [`result_is_current`] before applying anything.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// The per-job directory. A `fetch` writes the face and its licence here.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// `catalogue.tsv` or `import.tsv`, depending on the job.
    pub fn artifact_path(&self) -> &Path {
        &self.artifact
    }

    /// The helper's stderr, which is what a failure notice points the user at.
    pub fn log_path(&self) -> &Path {
        &self.log
    }

    /// Asks the helper to stop.
    ///
    /// `font_job_cancel` (`src/plug.c:1801-1809`). Note the asymmetry with
    /// Assist: this signals the **single process**, not its group, because
    /// `google_fonts.py` spawns no children of its own — it makes HTTP requests
    /// in-process. Reproduced rather than "improved" to a group signal, which
    /// would be a wider blast radius for no benefit.
    pub fn request_cancel(&mut self) -> io::Result<()> {
        self.cancelling = true;
        if let Some(child) = self.child.as_ref() {
            signal_process(child.id(), Signal::Term)?;
        }
        Ok(())
    }

    /// Whether a cancellation has been requested but not yet observed.
    pub fn is_cancelling(&self) -> bool {
        self.cancelling
    }

    /// Non-blocking progress check.
    ///
    /// `poll_font_job` (`src/plug.c:1811-1861`), in the order the C uses: the
    /// deadline is checked *before* the exit status, so a job that finished
    /// inside its last poll interval but after the deadline is still reported as
    /// a timeout. That ordering is preserved deliberately.
    pub fn poll(&mut self) -> io::Result<FontJobPoll> {
        let Some(child) = self.child.as_mut() else {
            return Ok(if self.cancelling {
                FontJobPoll::Cancelled
            } else {
                FontJobPoll::Failed
            });
        };

        if self.started.elapsed() >= self.kind.timeout() {
            // SIGKILL, not SIGTERM: the C does not negotiate with a job that
            // has already outrun its deadline (`src/plug.c:1832`).
            let outcome = kill_group_and_reap(child)?;
            self.child = None;
            let _ = outcome;
            return Ok(FontJobPoll::TimedOut);
        }

        match child.try_wait()? {
            None => Ok(FontJobPoll::Running),
            Some(status) => {
                self.child = None;
                if self.cancelling {
                    return Ok(FontJobPoll::Cancelled);
                }
                Ok(if status.success() {
                    FontJobPoll::Succeeded
                } else {
                    FontJobPoll::Failed
                })
            }
        }
    }

    /// Reads the job's artifact with a hard ceiling.
    ///
    /// `font_job_read_artifact` (`src/plug.c:1591-1615`). A worker that was
    /// killed while writing leaves a short or absent file, which reads as a
    /// failed job rather than a partial catalogue. Zero bytes and anything over
    /// [`CATALOGUE_MAX_BYTES`] are both rejected.
    pub fn read_artifact(&self) -> Result<String, FontJobError> {
        read_bounded_artifact(&self.artifact)
    }

    /// Reads and parses the catalogue a `catalogue` job produced.
    pub fn read_catalogue(&self) -> Result<FontCatalogue, FontJobError> {
        let text = self.read_artifact()?;
        Ok(FontCatalogue::parse(&text)?)
    }

    /// Reads and parses the manifest a `fetch` job produced.
    ///
    /// The digests in the result are still claims. Re-hash both files before
    /// anything is rasterized or written into a project.
    pub fn read_manifest(&self) -> Result<ImportManifest, FontJobError> {
        let text = self.read_artifact()?;
        Ok(ImportManifest::parse(&text)?)
    }

    /// Stops the helper and reaps it, reporting failure.
    ///
    /// Call this before dropping a job on shutdown. `Drop` does the same thing
    /// but cannot tell anyone how it went.
    pub fn shutdown(&mut self) -> io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        self.cancelling = true;
        signal_process(child.id(), Signal::Term)?;
        let polite = wait_bounded(
            &mut child,
            &ShutdownPolicy::with_grace(Duration::from_secs(2), Duration::from_millis(10)),
        )?;
        if polite.is_reaped() {
            return Ok(());
        }
        match kill_group_and_reap(&mut child)? {
            WaitOutcome::Reaped(_) => Ok(()),
            WaitOutcome::TimedOut => Err(io::Error::other(
                "the font helper could not be reaped after SIGKILL",
            )),
        }
    }
}

impl Drop for FontJob {
    fn drop(&mut self) {
        if self.child.is_some() {
            if let Err(error) = self.shutdown() {
                eprintln!("FONT: abandoned helper could not be reaped: {error}");
            }
        }
    }
}

/// Reads a helper artifact into memory with a hard ceiling
/// (`font_job_read_artifact`, `src/plug.c:1591-1615`).
fn read_bounded_artifact(path: &Path) -> Result<String, FontJobError> {
    let metadata = std::fs::metadata(path).map_err(|_| FontJobError::UnreadableArtifact)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > CATALOGUE_MAX_BYTES as u64 {
        return Err(FontJobError::UnreadableArtifact);
    }
    let bytes = std::fs::read(path).map_err(|_| FontJobError::UnreadableArtifact)?;
    // The C reads bytes and NUL-terminates; the parsers are byte-oriented and a
    // family name is validated as printable ASCII anyway, so invalid UTF-8
    // cannot reach a valid row. Rejecting it here keeps the parsers on `&str`.
    String::from_utf8(bytes).map_err(|_| FontJobError::UnreadableArtifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from ../musializer/tests/test_font_catalogue.c.
    const HEADER: &str = "musializer.font-catalogue/v1\t3\n";

    fn sample() -> String {
        format!(
            "{HEADER}\
             Roboto\tSans Serif\tcyrillic,greek,latin,latin-ext,vietnamese\n\
             Playfair Display\tSerif\tlatin,latin-ext\n\
             Space Mono\tMonospace\tlatin\n"
        )
    }

    #[test]
    fn parses_families_categories_and_the_scripts_we_can_render() {
        let catalogue = FontCatalogue::parse(&sample()).expect("parse");
        assert_eq!(catalogue.len(), 3);

        assert_eq!(catalogue.entries()[0].family, "Roboto");
        assert_eq!(catalogue.entries()[0].category, FontCategory::SansSerif);
        assert_eq!(
            catalogue.entries()[0].scripts,
            script::LATIN
                | script::LATIN_EXT
                | script::GREEK
                | script::CYRILLIC
                | script::VIETNAMESE
        );

        // The catalogue's own order is popularity order and must be preserved:
        // re-sorting it alphabetically would bury every face anyone wants.
        assert_eq!(catalogue.entries()[1].family, "Playfair Display");
        assert_eq!(catalogue.entries()[1].category, FontCategory::Serif);
        assert_eq!(
            catalogue.entries()[1].scripts,
            script::LATIN | script::LATIN_EXT
        );
        assert_eq!(catalogue.entries()[2].category, FontCategory::Monospace);
        assert_eq!(catalogue.entries()[2].scripts, script::LATIN);
    }

    #[test]
    fn tolerates_growth_it_does_not_understand() {
        // A category or a script this build has never heard of must not make the
        // whole catalogue unreadable. The family is still perfectly usable.
        let catalogue = FontCatalogue::parse(&format!(
            "{HEADER}Newface\tKinetic Variable\tlatin,tibetan,klingon\n"
        ))
        .expect("unknown growth is not an error");
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue.entries()[0].category, FontCategory::Unknown);
        assert_eq!(catalogue.entries()[0].scripts, script::LATIN);
        assert_eq!(FontCategory::Unknown.display_name(), "Other");
    }

    #[test]
    fn rejects_a_file_it_did_not_expect_and_keeps_the_old_one() {
        let mut catalogue = FontCatalogue::default();
        catalogue.parse_replace(&sample()).expect("seed");

        let bad: [(&str, CatalogueError); 9] = [
            ("", CatalogueError::Header),
            ("musializer.font-catalogue/v1", CatalogueError::Header),
            (
                "musializer.font-catalogue/v2\t1\nA\tSerif\tlatin\n",
                CatalogueError::Header,
            ),
            ("not a header at all\n", CatalogueError::Header),
            // Ragged rows: too few columns and too many.
            ("Roboto\tSans Serif\n", CatalogueError::Row),
            ("Roboto\tSans Serif\tlatin\textra\n", CatalogueError::Row),
            ("\tSans Serif\tlatin\n", CatalogueError::Family),
            // A control character would be drawn into the interface and handed
            // to the helper as an argument.
            ("Rob\u{01}oto\tSans Serif\tlatin\n", CatalogueError::Family),
            ("", CatalogueError::Empty),
        ];
        for (index, (text, expected)) in bad.iter().enumerate() {
            // The last four cases are rows that need the valid header prefixed;
            // the first four are whole documents.
            let document = if index >= 4 {
                format!("{HEADER}{text}")
            } else {
                (*text).to_string()
            };
            let error = catalogue
                .parse_replace(&document)
                .expect_err("must be rejected");
            assert_eq!(&error, expected, "case {index}: {document:?}");
            // Whatever went wrong, the catalogue already loaded is still there.
            // A failed refresh must not empty a working picker.
            assert_eq!(catalogue.len(), 3);
            assert_eq!(catalogue.entries()[0].family, "Roboto");
        }
    }

    #[test]
    fn refuses_a_family_name_longer_than_it_can_hold() {
        // Exactly at capacity is the longest usable name.
        let longest = "A".repeat(FAMILY_MAX_BYTES);
        let catalogue = FontCatalogue::parse(&format!("{HEADER}{longest}\tSerif\tlatin\n"))
            .expect("the longest name is usable");
        assert_eq!(catalogue.entries()[0].family.len(), FAMILY_MAX_BYTES);

        let too_long = "A".repeat(FAMILY_MAX_BYTES + 1);
        assert_eq!(
            FontCatalogue::parse(&format!("{HEADER}{too_long}\tSerif\tlatin\n")).unwrap_err(),
            CatalogueError::Family
        );
    }

    #[test]
    fn accepts_a_final_row_without_a_trailing_newline() {
        let catalogue = FontCatalogue::parse(&format!(
            "{HEADER}Roboto\tSans Serif\tlatin\nInter\tSans Serif\tlatin"
        ))
        .expect("parse");
        assert_eq!(catalogue.len(), 2);
        assert_eq!(catalogue.entries()[1].family, "Inter");

        // And blank rows in the middle are skipped rather than counted.
        let catalogue = FontCatalogue::parse(&format!(
            "{HEADER}Roboto\tSans Serif\tlatin\n\n\nInter\tSans Serif\tlatin\n"
        ))
        .expect("parse");
        assert_eq!(catalogue.len(), 2);
    }

    #[test]
    fn an_oversized_catalogue_is_refused_without_being_parsed() {
        // The ceiling exists because another process writes this file.
        let mut text = String::from(HEADER);
        text.push_str(&"A".repeat(CATALOGUE_MAX_BYTES));
        assert_eq!(
            FontCatalogue::parse(&text).unwrap_err(),
            CatalogueError::TooLarge
        );
        assert_eq!(
            ImportManifest::parse(&text).unwrap_err(),
            CatalogueError::TooLarge
        );
    }

    #[test]
    fn too_many_families_is_a_capacity_error_not_a_truncated_list() {
        let mut text = String::from(HEADER);
        for index in 0..=CATALOGUE_MAX_FAMILIES {
            text.push_str(&format!("F{index}\tSerif\tlatin\n"));
        }
        assert_eq!(
            FontCatalogue::parse(&text).unwrap_err(),
            CatalogueError::Capacity
        );
    }

    #[test]
    fn filter_folds_case_and_reports_more_than_it_writes() {
        let catalogue = FontCatalogue::parse(&sample()).expect("parse");
        let mut indices = [0usize; 8];

        let count = catalogue.filter("", 0, &mut indices);
        assert_eq!((count.matched, count.written), (3, 3));

        let count = catalogue.filter("SPACE", 0, &mut indices);
        assert_eq!(count.matched, 1);
        assert_eq!(indices[0], 2);
        let count = catalogue.filter("air dis", 0, &mut indices);
        assert_eq!(count.matched, 1);
        assert_eq!(indices[0], 1);
        let count = catalogue.filter("zzz", 0, &mut indices);
        assert_eq!((count.matched, count.written), (0, 0));

        // Only Roboto carries Cyrillic, so requiring it must exclude the rest
        // rather than offer a face that would render the lyric as empty boxes.
        let count = catalogue.filter("", script::CYRILLIC, &mut indices);
        assert_eq!(count.matched, 1);
        assert_eq!(indices[0], 0);
        let count = catalogue.filter("", script::CYRILLIC | script::GREEK, &mut indices);
        assert_eq!(count.matched, 1);

        // A caller with room for one still learns that three matched, which is
        // what lets the picker say "showing 1 of 3" instead of truncating.
        let mut one = [0usize; 1];
        let count = catalogue.filter("", 0, &mut one);
        assert_eq!((count.matched, count.written), (3, 1));
        let count = catalogue.filter("", 0, &mut []);
        assert_eq!((count.matched, count.written), (3, 0));
    }

    #[test]
    fn find_matches_a_family_exactly() {
        let catalogue = FontCatalogue::parse(&sample()).expect("parse");
        assert_eq!(catalogue.find("Space Mono"), Some(2));
        // A saved project names one family. Resolving "space mono" or "Space" to
        // a different face would silently retypeset someone's captions.
        assert_eq!(catalogue.find("space mono"), None);
        assert_eq!(catalogue.find("Space"), None);
        assert_eq!(catalogue.find(""), None);
        assert_eq!(catalogue.find("Roboto"), Some(0));
    }

    const MANIFEST_HEADER: &str = "musializer.font-import/v1\t1\n";
    const GOOD_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn manifest_reads_one_row_and_refuses_anything_else() {
        let good = format!(
            "{MANIFEST_HEADER}Space Mono\t/tmp/j/spacemono.ttf\t{GOOD_DIGEST}\
             \t/tmp/j/spacemono.licence.txt\t{GOOD_DIGEST}\tOFL-1.1\n"
        );
        let manifest = ImportManifest::parse(&good).expect("parse");
        assert_eq!(manifest.family, "Space Mono");
        assert_eq!(manifest.font_path, "/tmp/j/spacemono.ttf");
        assert_eq!(manifest.font_sha256, GOOD_DIGEST);
        assert_eq!(manifest.licence_name, "OFL-1.1");

        let bad = [
            String::new(),
            "musializer.font-import/v1".to_string(),
            MANIFEST_HEADER.to_string(),
            format!("musializer.font-import/v2\t1\nA\t/a\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\n"),
            // Too few columns, and too many.
            format!("{MANIFEST_HEADER}A\t/a\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\n"),
            format!("{MANIFEST_HEADER}A\t/a\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\textra\n"),
            // A digest that is short, uppercase, or not hex at all. Accepting
            // one would mean comparing it against a real hash and always
            // failing, with the confusing message that the download did not
            // match.
            format!("{MANIFEST_HEADER}A\t/a\tabc\t/b\t{GOOD_DIGEST}\tOFL\n"),
            format!("{MANIFEST_HEADER}A\t/a\t{GOOD_DIGEST}\t/b\tZZZ{GOOD_DIGEST}\tOFL\n"),
            format!(
                "{MANIFEST_HEADER}A\t/a\t{}\t/b\t{GOOD_DIGEST}\tOFL\n",
                GOOD_DIGEST.to_uppercase()
            ),
            // Empty family, path, or licence name.
            format!("{MANIFEST_HEADER}\t/a\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\n"),
            format!("{MANIFEST_HEADER}A\t\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\n"),
            format!("{MANIFEST_HEADER}A\t/a\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\t\n"),
        ];
        for text in &bad {
            assert!(
                ImportManifest::parse(text).is_err(),
                "should have been refused: {text:?}"
            );
        }
        // And the caller still holds the import it already had: Rust gives this
        // for free, where the C had to build into a local to get it.
        assert_eq!(manifest.family, "Space Mono");
    }

    #[test]
    fn manifest_refuses_a_path_longer_than_it_can_hold() {
        let path = "p".repeat(IMPORT_PATH_MAX_BYTES + 1);
        let text = format!("{MANIFEST_HEADER}A\t{path}\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\n");
        assert_eq!(
            ImportManifest::parse(&text).unwrap_err(),
            CatalogueError::Row
        );
        // Exactly at capacity is accepted.
        let path = "p".repeat(IMPORT_PATH_MAX_BYTES);
        let text = format!("{MANIFEST_HEADER}A\t{path}\t{GOOD_DIGEST}\t/b\t{GOOD_DIGEST}\tOFL\n");
        assert!(ImportManifest::parse(&text).is_ok());
    }

    #[test]
    fn scripts_describe_names_coverage_and_never_overruns() {
        assert_eq!(
            describe_scripts(script::LATIN | script::GREEK),
            "Latin, Greek"
        );
        assert_eq!(describe_scripts(0), "no known script");

        // A buffer too small keeps whole names rather than half of one.
        assert_eq!(
            describe_scripts_bounded(script::LATIN | script::CYRILLIC, 8),
            "Latin"
        );
        // Room for nothing at all is an empty list, not the phrase that means
        // the face covers nothing.
        assert_eq!(describe_scripts_bounded(script::LATIN, 2), "");
        assert_eq!(describe_scripts_bounded(script::LATIN, 0), "");
        assert_eq!(
            describe_scripts_bounded(script::LATIN | script::GREEK, 64),
            "Latin, Greek"
        );
    }

    #[test]
    fn nonces_skip_zero_and_gate_stale_results() {
        assert_eq!(next_nonce(0), 1);
        assert_eq!(next_nonce(7), 8);
        // Wrapping past u64::MAX must not produce the "no job" value.
        assert_eq!(next_nonce(u64::MAX), 1);

        assert!(result_is_current(4, 4));
        assert!(!result_is_current(3, 4), "a superseded job must not apply");
        assert!(!result_is_current(0, 0), "zero is never a live job");
    }

    // -----------------------------------------------------------------------
    // Supervision. Exercised with a fake helper script, because the real one
    // would contact fonts.google.com and this suite never touches the network.
    // -----------------------------------------------------------------------

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("musializer-font-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch");
            Scratch(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// A stand-in for `tools/google_fonts.py`. It is spawned through
        /// `python3` exactly as the real helper is, so the argv shape is under
        /// test too — hence a Python file rather than a shell script.
        fn fake_helper(&self, label: &str, body: &str) -> PathBuf {
            let path = self.join(label);
            std::fs::write(&path, format!("import sys, time, pathlib, os\n{body}\n"))
                .expect("helper");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn poll_until_finished(job: &mut FontJob) -> FontJobPoll {
        for _ in 0..600 {
            match job.poll().expect("poll") {
                FontJobPoll::Running => std::thread::sleep(Duration::from_millis(10)),
                other => return other,
            }
        }
        panic!("the fake helper never finished");
    }

    #[test]
    fn a_catalogue_job_writes_a_manifest_the_reader_accepts() {
        let scratch = Scratch::new("catalogue");
        // argv: <helper> catalogue <cache> --index <artifact>
        let helper = scratch.fake_helper(
            "helper.py",
            "assert sys.argv[1] == 'catalogue'\n\
             assert sys.argv[3] == '--index'\n\
             pathlib.Path(sys.argv[4]).write_text(\
             'musializer.font-catalogue/v1\\t1\\nInter\\tSans Serif\\tlatin\\n')",
        );
        let mut job =
            FontJob::start(&helper, &scratch.0, FontJobKind::Catalogue, "", 0).expect("start");
        assert_eq!(job.nonce(), 1);
        assert!(job.artifact_path().ends_with("catalogue.tsv"));
        assert_eq!(poll_until_finished(&mut job), FontJobPoll::Succeeded);

        let catalogue = job.read_catalogue().expect("read");
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue.entries()[0].family, "Inter");
    }

    #[test]
    fn a_fetch_job_is_given_the_family_and_its_own_directory() {
        let scratch = Scratch::new("fetch");
        // argv: <helper> fetch <family> <directory>
        let helper = scratch.fake_helper(
            "helper.py",
            &format!(
                "assert sys.argv[1] == 'fetch'\n\
                 family = sys.argv[2]\n\
                 directory = pathlib.Path(sys.argv[3])\n\
                 digest = '{GOOD_DIGEST}'\n\
                 (directory / 'import.tsv').write_text(\n\
                 'musializer.font-import/v1\\t1\\n' + family + '\\t/f.ttf\\t' + digest +\n\
                 '\\t/l.txt\\t' + digest + '\\tOFL-1.1\\n')"
            ),
        );
        let mut job = FontJob::start(&helper, &scratch.0, FontJobKind::Fetch, "Space Mono", 41)
            .expect("start");
        assert_eq!(job.nonce(), 42);
        assert!(job.artifact_path().ends_with(MANIFEST_INDEX_NAME));
        // Every job writes into its own nonce-named directory, so a superseded
        // worker writes where nothing will read.
        assert!(job.directory().ends_with(format!("{:016x}", 42)));
        assert_eq!(poll_until_finished(&mut job), FontJobPoll::Succeeded);

        let manifest = job.read_manifest().expect("read");
        assert_eq!(manifest.family, "Space Mono");
        assert_eq!(manifest.licence_name, "OFL-1.1");
    }

    #[test]
    fn a_fetch_without_a_family_never_spawns_anything() {
        let scratch = Scratch::new("nofamily");
        let helper = scratch.fake_helper("helper.py", "raise SystemExit('should not run')");
        let error = FontJob::start(&helper, &scratch.0, FontJobKind::Fetch, "", 0)
            .expect_err("a fetch needs a family");
        assert!(matches!(error, FontJobError::NoFamily), "{error:?}");
    }

    #[test]
    fn a_failing_helper_is_reported_and_its_log_is_kept() {
        let scratch = Scratch::new("failing");
        let helper = scratch.fake_helper(
            "helper.py",
            "sys.stderr.write('no network\\n')\nraise SystemExit(1)",
        );
        let mut job =
            FontJob::start(&helper, &scratch.0, FontJobKind::Catalogue, "", 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), FontJobPoll::Failed);
        // The failure notice points at the log, so the log must be there.
        let log = std::fs::read_to_string(job.log_path()).expect("log");
        assert!(log.contains("no network"), "log was {log:?}");
        // And no artifact means the reader refuses rather than inventing one.
        assert!(matches!(
            job.read_catalogue(),
            Err(FontJobError::UnreadableArtifact)
        ));
    }

    #[test]
    fn a_truncated_artifact_reads_as_a_failed_job() {
        let scratch = Scratch::new("truncated");
        // Writes nothing at all, as a worker killed mid-write would leave it.
        let helper = scratch.fake_helper("helper.py", "pathlib.Path(sys.argv[4]).write_text('')");
        let mut job =
            FontJob::start(&helper, &scratch.0, FontJobKind::Catalogue, "", 0).expect("start");
        assert_eq!(poll_until_finished(&mut job), FontJobPoll::Succeeded);
        assert!(matches!(
            job.read_artifact(),
            Err(FontJobError::UnreadableArtifact)
        ));
    }

    #[test]
    fn cancelling_stops_the_helper_and_reports_it_as_cancelled() {
        let scratch = Scratch::new("cancel");
        let helper = scratch.fake_helper("helper.py", "time.sleep(60)");
        let mut job =
            FontJob::start(&helper, &scratch.0, FontJobKind::Catalogue, "", 0).expect("start");
        assert_eq!(job.poll().expect("poll"), FontJobPoll::Running);
        job.request_cancel().expect("cancel");
        assert!(job.is_cancelling());
        assert_eq!(poll_until_finished(&mut job), FontJobPoll::Cancelled);
    }

    #[test]
    fn dropping_a_running_job_still_reaps_the_helper() {
        let scratch = Scratch::new("drop");
        let helper = scratch.fake_helper("helper.py", "time.sleep(60)");
        let pid;
        {
            let mut job =
                FontJob::start(&helper, &scratch.0, FontJobKind::Catalogue, "", 0).expect("start");
            pid = job.child.as_ref().expect("child").id();
            assert_eq!(job.poll().expect("poll"), FontJobPoll::Running);
        }
        // Reaped means gone from /proc entirely, not a zombie.
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "Drop must reap the helper, not leave a zombie"
        );
    }

    #[test]
    fn the_helper_candidate_list_probes_both_layouts_in_order() {
        let scratch = Scratch::new("resolve");
        // Nothing anywhere: not found.
        assert_eq!(find_font_helper(&scratch.0), None);

        // The extracted-distribution layout: <appdir>/tools/google_fonts.py.
        let distribution = scratch.join("dist");
        std::fs::create_dir_all(distribution.join("tools")).unwrap();
        std::fs::write(distribution.join("tools/google_fonts.py"), "").unwrap();
        assert_eq!(
            find_font_helper(&distribution),
            Some(distribution.join("tools/google_fonts.py"))
        );

        // The ./build source-run layout: <appdir>/../tools/google_fonts.py.
        let source = scratch.join("source");
        std::fs::create_dir_all(source.join("tools")).unwrap();
        std::fs::create_dir_all(source.join("build")).unwrap();
        std::fs::write(source.join("tools/google_fonts.py"), "").unwrap();
        assert_eq!(
            find_font_helper(&source.join("build")),
            Some(source.join("build/../tools/google_fonts.py"))
        );

        // The Assist helper resolves the same way with a different filename.
        std::fs::write(distribution.join("tools/external_analysis.py"), "").unwrap();
        assert_eq!(
            find_assist_helper(&distribution),
            Some(distribution.join("tools/external_analysis.py"))
        );
    }

    #[test]
    fn a_bad_helper_override_fails_hard_instead_of_falling_back() {
        // Documented in docs/PHASE0_INVENTORY.md section 9 and worth a test,
        // because "fall back to the probe" is the obvious wrong fix. Uses a
        // variable name the application never reads so no other test is
        // disturbed by the process-wide mutation.
        let scratch = Scratch::new("override");
        let distribution = scratch.join("dist");
        std::fs::create_dir_all(distribution.join("tools")).unwrap();
        let real = distribution.join("tools/google_fonts.py");
        std::fs::write(&real, "").unwrap();

        let variable = "MUSIALIZER_FONT_HELPER_TEST_ONLY";
        std::env::set_var(variable, scratch.join("absent.py"));
        assert_eq!(
            find_helper(&distribution, variable, "google_fonts.py"),
            None,
            "a set-but-missing override must not fall back to the probe"
        );

        std::env::set_var(variable, &real);
        assert_eq!(
            find_helper(&distribution, variable, "google_fonts.py"),
            Some(real.clone())
        );

        // An empty override is treated as unset, so the probe runs.
        std::env::set_var(variable, "");
        assert_eq!(
            find_helper(&distribution, variable, "google_fonts.py"),
            Some(real)
        );
        std::env::remove_var(variable);
    }

    #[test]
    fn job_timeouts_are_the_oracles_two_values() {
        assert_eq!(FontJobKind::Catalogue.timeout(), Duration::from_secs(120));
        assert_eq!(FontJobKind::Fetch.timeout(), Duration::from_secs(180));
    }
}
