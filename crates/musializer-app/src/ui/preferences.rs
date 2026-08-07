//! Per-user shell scale and split preferences.
//!
//! These values describe the operator's workstation, not a music project, so
//! they deliberately live outside `.musi`. A corrupt file is an error rather
//! than an empty preference set; the application can keep running with defaults
//! but must refuse to overwrite evidence the user may be able to repair.
//!
//! [`recent`] is the second per-user store and lives beside this one rather than
//! inside it; its module comment says why.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::scale::UiScalePreference;

const SCHEMA: &str = "musializer.ui-preferences/v1";
const MAX_FILE_SIZE: u64 = 64 * 1024;

/// What a persisted lyric cue lane height may be (LX1-d).
///
/// Deliberately wider than the panel's own 33..66 clamp and checked separately
/// from the split widths: a lane is tens of pixels, not hundreds, so the shared
/// `80.0..=4096.0` bound would reject every value this field can legitimately
/// hold. The panel clamps again against the window it is drawn in, so this range
/// only has to exclude values that are not a lane at all.
const LANE_HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 8.0..=256.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiPreferences {
    pub scale: UiScalePreference,
    pub sidebar_width: Option<f32>,
    pub inspector_width: Option<f32>,
    pub timeline_height: Option<f32>,
    /// The lyric editor's cue lane height, dragged by its bottom edge (LX1-d).
    ///
    /// Here rather than in the `.musi` file for this module's stated reason: it
    /// describes the operator's screen, not the music. A resize the user has to
    /// redo every launch is the friction the resize was asked for to remove.
    pub lyric_lane_height: Option<f32>,
}

impl UiPreferences {
    #[must_use]
    pub fn sane(self) -> bool {
        [
            self.sidebar_width,
            self.inspector_width,
            self.timeline_height,
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && (80.0..=4096.0).contains(&value))
            // `into_iter().all` rather than `is_none_or`, which is stable only
            // since 1.82 and this workspace's MSRV is 1.80.
            && self
                .lyric_lane_height
                .into_iter()
                .all(|value| value.is_finite() && LANE_HEIGHT_RANGE.contains(&value))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UiPreferencesError {
    #[error("could not read the preference file")]
    Read,
    #[error("the preference file has an invalid format")]
    Format,
    #[error("could not create the preference directory")]
    Directory,
    #[error("could not write the preference file")]
    Write,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema: String,
    scale: String,
    sidebar_width: Option<f32>,
    inspector_width: Option<f32>,
    timeline_height: Option<f32>,
    /// `default` rather than required, and that is the compatibility contract
    /// rather than a convenience: every `ui.json` this application has already
    /// written carries exactly the four fields above, and a fifth required one
    /// would make all of them fail to parse — which this module treats as
    /// corruption and refuses to overwrite, so the user would lose their splits
    /// *and* be told the file is broken.
    #[serde(default)]
    lyric_lane_height: Option<f32>,
}

#[must_use]
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MUSIALIZER_UI_PREFERENCES") {
        return (!path.is_empty()).then(|| PathBuf::from(path));
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer/ui.json"));
    }
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".config/musializer/ui.json"))
}

pub fn load(path: &Path) -> Result<Option<UiPreferences>, UiPreferencesError> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UiPreferencesError::Read),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UiPreferencesError::Read)?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(UiPreferencesError::Format);
    }
    let document: Document =
        serde_json::from_slice(&bytes).map_err(|_| UiPreferencesError::Format)?;
    if document.schema != SCHEMA {
        return Err(UiPreferencesError::Format);
    }
    let scale = UiScalePreference::parse(&document.scale).ok_or(UiPreferencesError::Format)?;
    let preferences = UiPreferences {
        scale,
        sidebar_width: document.sidebar_width,
        inspector_width: document.inspector_width,
        timeline_height: document.timeline_height,
        lyric_lane_height: document.lyric_lane_height,
    };
    preferences
        .sane()
        .then_some(Some(preferences))
        .ok_or(UiPreferencesError::Format)
}

pub fn save(path: &Path, preferences: UiPreferences) -> Result<(), UiPreferencesError> {
    if !preferences.sane() {
        return Err(UiPreferencesError::Format);
    }
    let document = Document {
        schema: SCHEMA.to_string(),
        scale: match preferences.scale {
            UiScalePreference::Auto => "auto".to_string(),
            UiScalePreference::Fixed(scale) => scale.percent().to_string(),
        },
        sidebar_width: preferences.sidebar_width,
        inspector_width: preferences.inspector_width,
        timeline_height: preferences.timeline_height,
        lyric_lane_height: preferences.lyric_lane_height,
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|_| UiPreferencesError::Format)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| UiPreferencesError::Directory)?;
    }
    musializer_runtime::process::publish::atomic_write(path, &bytes)
        .map_err(|_| UiPreferencesError::Write)
}

#[cfg(test)]
mod tests {
    use super::super::scale::UI_SCALE_STEPS;
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../build/test-ui-preferences")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn preferences_round_trip_without_entering_a_project() {
        let path = scratch("round-trip").join("nested/ui.json");
        let preferences = UiPreferences {
            scale: UiScalePreference::parse("150").unwrap(),
            sidebar_width: Some(384.0),
            inspector_width: Some(420.0),
            timeline_height: Some(460.0),
            lyric_lane_height: Some(48.0),
        };
        save(&path, preferences).unwrap();
        assert_eq!(load(&path).unwrap(), Some(preferences));
    }

    #[test]
    fn a_file_written_before_the_lane_was_resizable_still_opens() {
        // LX1-d's compatibility contract, stated as a test rather than as a
        // comment: this is the exact document a build before the resize wrote.
        let path = scratch("pre-lane").join("ui.json");
        std::fs::write(
            &path,
            br#"{"schema":"musializer.ui-preferences/v1","scale":"auto",
                 "sidebar_width":300.0,"inspector_width":null,"timeline_height":null}"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap().expect("an older file still parses");
        assert_eq!(loaded.sidebar_width, Some(300.0));
        assert_eq!(loaded.lyric_lane_height, None);
    }

    #[test]
    fn a_lane_height_outside_its_own_range_is_refused_rather_than_clamped() {
        // The split widths' 80 px floor must not be applied to a lane: 33 is the
        // lane's *default*, and a shared bound would reject every honest value.
        let path = scratch("lane-range").join("ui.json");
        for accepted in [8.0f32, 33.0, 66.0, 256.0] {
            let preferences = UiPreferences {
                lyric_lane_height: Some(accepted),
                ..UiPreferences::default()
            };
            assert!(preferences.sane(), "{accepted} is a lane height");
            save(&path, preferences).unwrap();
            assert_eq!(load(&path).unwrap(), Some(preferences));
        }
        for refused in [0.0f32, -33.0, 257.0, f32::NAN] {
            let preferences = UiPreferences {
                lyric_lane_height: Some(refused),
                ..UiPreferences::default()
            };
            assert!(!preferences.sane(), "{refused} is not a lane height");
            assert_eq!(save(&path, preferences), Err(UiPreferencesError::Format));
        }
    }

    #[test]
    fn corrupt_preferences_are_not_treated_as_an_empty_store() {
        let path = scratch("corrupt").join("ui.json");
        std::fs::write(&path, b"{ broken").unwrap();
        assert_eq!(load(&path), Err(UiPreferencesError::Format));
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");
    }

    #[test]
    fn every_supported_scale_serializes() {
        for scale in UI_SCALE_STEPS {
            let preferences = UiPreferences {
                scale: UiScalePreference::Fixed(super::super::scale::UiScale::new(scale).unwrap()),
                ..UiPreferences::default()
            };
            let path = scratch(&format!("scale-{scale}")).join("ui.json");
            save(&path, preferences).unwrap();
            assert_eq!(load(&path).unwrap(), Some(preferences));
        }
    }
}

/// The welcome screen's recent-project list (UX0-C06).
///
/// # Why a second file rather than a field in [`UiPreferences`]
///
/// Three reasons, and the first two are the failure-tolerance requirement rather
/// than tidiness:
///
/// - **Corruption must not be contagious.** `ui.json` and this list are both
///   refused wholesale when they do not parse, because refusing is what stops the
///   application overwriting evidence the user might repair. Folding the list in
///   would mean one truncated write costs the operator their splits *and* their
///   history, and the surviving half would be unrecoverable rather than unread.
/// - **The write cadences are unrelated.** `ui.json` is rewritten whenever a
///   split is dragged; this list changes when a project is opened or forgotten.
/// - [`UiPreferences`] is `Copy` and travels by value inside
///   `ShellCommand::SaveUiPreferences`. A `Vec` of entries cannot be, and making
///   that command clone a list that did not change on every drag would be paying
///   for the coupling twice.
///
/// Everything else is deliberately identical to its neighbour: a versioned schema
/// string checked exactly, a size bound, an atomic replace, and a load that
/// distinguishes "no file yet" from "a file I refuse to touch".
pub mod recent {
    use std::io::{ErrorKind, Read};
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Serialize};

    const SCHEMA: &str = "musializer.recent-projects/v1";
    const MAX_FILE_SIZE: u64 = 64 * 1024;

    /// How many entries are kept, and how many the welcome screen will ever draw.
    ///
    /// Small on purpose. This is the first thirty seconds of a session, not a
    /// file manager: a list long enough to need reading is one the user scans
    /// instead of recognising.
    pub const CAPACITY: usize = 8;

    /// Bounds on a single entry, so a hostile or truncated file cannot make the
    /// welcome screen draw a megabyte of text.
    const MAX_NAME_BYTES: usize = 160;
    const MAX_PATH_BYTES: usize = 4096;

    /// One remembered project.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct RecentProject {
        /// Absolute where the caller could make it so; compared verbatim for
        /// de-duplication, which is why normalizing is the caller's job.
        pub path: PathBuf,
        /// What to call it on screen. Recorded while the project was open, so it
        /// is the project's own title rather than a filename guessed later.
        pub name: String,
        /// Seconds since the Unix epoch. `None` when the writing build had no
        /// readable clock, which draws as no age rather than as 1970.
        pub opened_unix: Option<i64>,
        /// Derived, never serialized: whether the file was on disk the last time
        /// [`RecentProjects::probe`] looked.
        ///
        /// Held on the entry rather than recomputed while drawing, because a
        /// `stat` per row per frame is filesystem work inside a render loop, and
        /// because a row that flickered between present and missing as a network
        /// mount stalled would be worse than a stale answer.
        pub missing: bool,
    }

    impl RecentProject {
        fn sane(&self) -> bool {
            let path = self.path.as_os_str();
            !path.is_empty()
                && path.len() <= MAX_PATH_BYTES
                && !self.name.trim().is_empty()
                && self.name.len() <= MAX_NAME_BYTES
                && !self.name.contains(['\n', '\r'])
        }
    }

    /// The list, most recently opened first.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct RecentProjects {
        entries: Vec<RecentProject>,
    }

    impl RecentProjects {
        #[must_use]
        pub fn entries(&self) -> &[RecentProject] {
            &self.entries
        }

        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.entries.is_empty()
        }

        #[must_use]
        pub fn len(&self) -> usize {
            self.entries.len()
        }

        /// Puts `path` at the front, replacing any earlier entry for it.
        ///
        /// Replacing rather than appending is what keeps "recent" honest: opening
        /// the same project twice should move it up, not fill the list with one
        /// name. De-duplication is verbatim `PathBuf` equality — the caller
        /// normalizes, because doing it here would mean touching the filesystem
        /// from a type whose whole point is that it does not.
        pub fn record(&mut self, path: PathBuf, name: String, opened_unix: Option<i64>) {
            self.entries.retain(|entry| entry.path != path);
            self.entries.insert(
                0,
                RecentProject {
                    path,
                    name,
                    opened_unix,
                    missing: false,
                },
            );
            self.entries.truncate(CAPACITY);
        }

        /// Drops the entry for `path`. `true` when something was removed.
        pub fn remove(&mut self, path: &Path) -> bool {
            let before = self.entries.len();
            self.entries.retain(|entry| entry.path != path);
            self.entries.len() != before
        }

        /// Refreshes every entry's [`RecentProject::missing`] flag.
        ///
        /// Takes the existence test as a closure so the policy stays testable
        /// without a filesystem; the application passes `|path| path.is_file()`.
        pub fn probe(&mut self, exists: impl Fn(&Path) -> bool) {
            for entry in &mut self.entries {
                entry.missing = !exists(&entry.path);
            }
        }
    }

    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum RecentError {
        #[error("could not read the recent-projects file")]
        Read,
        #[error("the recent-projects file has an invalid format")]
        Format,
        #[error("could not create the configuration directory")]
        Directory,
        #[error("could not write the recent-projects file")]
        Write,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Document {
        schema: String,
        projects: Vec<Entry>,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        path: String,
        name: String,
        opened_unix: Option<i64>,
    }

    /// `$MUSIALIZER_RECENT_PROJECTS`, else the XDG config directory.
    ///
    /// The environment override exists for the same reason
    /// `MUSIALIZER_UI_PREFERENCES` does, and it is what lets the headless gate
    /// photograph a populated list without writing into the operator's own
    /// configuration.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("MUSIALIZER_RECENT_PROJECTS") {
            return (!path.is_empty()).then(|| PathBuf::from(path));
        }
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            return (!path.is_empty()).then(|| PathBuf::from(path).join("musializer/recent.json"));
        }
        std::env::var_os("HOME")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join(".config/musializer/recent.json"))
    }

    /// `Ok(None)` is "no file yet", which is a new user rather than a fault.
    pub fn load(path: &Path) -> Result<Option<RecentProjects>, RecentError> {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(RecentError::Read),
        };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_SIZE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RecentError::Read)?;
        if bytes.len() as u64 > MAX_FILE_SIZE {
            return Err(RecentError::Format);
        }
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| RecentError::Format)?;
        if document.schema != SCHEMA {
            return Err(RecentError::Format);
        }
        if document.projects.len() > CAPACITY {
            return Err(RecentError::Format);
        }
        let mut list = RecentProjects::default();
        for entry in document.projects {
            let entry = RecentProject {
                path: PathBuf::from(entry.path),
                name: entry.name,
                opened_unix: entry.opened_unix,
                missing: false,
            };
            if !entry.sane() {
                return Err(RecentError::Format);
            }
            // A file that names one project twice is a file this application did
            // not write, and silently collapsing it would mean saving back
            // something the user never had.
            if list.entries.iter().any(|kept| kept.path == entry.path) {
                return Err(RecentError::Format);
            }
            list.entries.push(entry);
        }
        Ok(Some(list))
    }

    pub fn save(path: &Path, list: &RecentProjects) -> Result<(), RecentError> {
        if list.entries.len() > CAPACITY || !list.entries.iter().all(RecentProject::sane) {
            return Err(RecentError::Format);
        }
        let document = Document {
            schema: SCHEMA.to_string(),
            projects: list
                .entries
                .iter()
                .map(|entry| Entry {
                    path: entry.path.to_string_lossy().into_owned(),
                    name: entry.name.clone(),
                    opened_unix: entry.opened_unix,
                })
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&document).map_err(|_| RecentError::Format)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| RecentError::Directory)?;
        }
        musializer_runtime::process::publish::atomic_write(path, &bytes)
            .map_err(|_| RecentError::Write)
    }

    /// Seconds since the Unix epoch, or `None` when the clock is before it.
    #[must_use]
    pub fn now_unix() -> Option<i64> {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
    }

    /// "3 days ago", for the welcome screen's recent column.
    ///
    /// Relative rather than a calendar date, and that is a dependency decision as
    /// much as a design one: rendering `2026-08-06` needs a civil-calendar
    /// conversion, and this workspace is not adding a date crate to put three
    /// words on a screen. Relative is also the more useful answer here — a
    /// returning user is asking "which one was I working on", not "what was the
    /// date".
    ///
    /// A clock that has gone backwards since the entry was written reads as "just
    /// now" rather than as a negative age, because the alternative is a list that
    /// appears to contain the future.
    #[must_use]
    pub fn describe_age(opened_unix: Option<i64>, now_unix: Option<i64>) -> Option<String> {
        let (opened, now) = (opened_unix?, now_unix?);
        let seconds = now.saturating_sub(opened).max(0);
        // Plain integer arithmetic on a fixed-length day. Nothing here needs to
        // survive a leap second, and a wrong answer is a word, not a defect.
        const MINUTE: i64 = 60;
        const HOUR: i64 = 60 * MINUTE;
        const DAY: i64 = 24 * HOUR;
        const WEEK: i64 = 7 * DAY;
        const MONTH: i64 = 30 * DAY;
        const YEAR: i64 = 365 * DAY;
        fn plural(count: i64, unit: &str) -> String {
            if count == 1 {
                format!("1 {unit} ago")
            } else {
                format!("{count} {unit}s ago")
            }
        }
        Some(if seconds < MINUTE {
            "just now".to_string()
        } else if seconds < HOUR {
            plural(seconds / MINUTE, "minute")
        } else if seconds < DAY {
            plural(seconds / HOUR, "hour")
        } else if seconds < 2 * DAY {
            "yesterday".to_string()
        } else if seconds < WEEK {
            plural(seconds / DAY, "day")
        } else if seconds < MONTH {
            plural(seconds / WEEK, "week")
        } else if seconds < YEAR {
            plural(seconds / MONTH, "month")
        } else {
            plural(seconds / YEAR, "year")
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn scratch(name: &str) -> PathBuf {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../build/test-recent-projects")
                .join(name);
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            root
        }

        fn list(paths: &[&str]) -> RecentProjects {
            let mut list = RecentProjects::default();
            // Recorded oldest first, so the front of the list is the last one in.
            for (index, path) in paths.iter().enumerate() {
                list.record(
                    PathBuf::from(path),
                    format!("Project {index}"),
                    Some(1_700_000_000 + index as i64),
                );
            }
            list
        }

        #[test]
        fn the_most_recently_opened_project_is_first() {
            let list = list(&["/a.musi", "/b.musi", "/c.musi"]);
            let order: Vec<_> = list
                .entries()
                .iter()
                .map(|entry| entry.path.to_string_lossy().into_owned())
                .collect();
            assert_eq!(order, ["/c.musi", "/b.musi", "/a.musi"]);
        }

        #[test]
        fn reopening_a_project_moves_it_up_instead_of_duplicating_it() {
            let mut list = list(&["/a.musi", "/b.musi", "/c.musi"]);
            list.record(PathBuf::from("/a.musi"), "Renamed".to_string(), Some(9));
            assert_eq!(list.len(), 3);
            assert_eq!(list.entries()[0].path, PathBuf::from("/a.musi"));
            // The freshest name and time win, which is what stops a project
            // renamed on disk showing its old title forever.
            assert_eq!(list.entries()[0].name, "Renamed");
            assert_eq!(list.entries()[0].opened_unix, Some(9));
        }

        #[test]
        fn the_list_stops_at_its_capacity_and_drops_the_oldest() {
            let paths: Vec<String> = (0..CAPACITY + 4).map(|i| format!("/p{i}.musi")).collect();
            let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
            let list = list(&borrowed);
            assert_eq!(list.len(), CAPACITY);
            assert_eq!(list.entries()[0].path, PathBuf::from("/p11.musi"));
            // The four oldest are gone, not merely off the end of the draw.
            for dropped in 0..4 {
                let path = PathBuf::from(format!("/p{dropped}.musi"));
                assert!(list.entries().iter().all(|entry| entry.path != path));
            }
        }

        #[test]
        fn removing_an_entry_reports_whether_it_was_there() {
            let mut list = list(&["/a.musi", "/b.musi"]);
            assert!(list.remove(Path::new("/a.musi")));
            assert!(!list.remove(Path::new("/a.musi")));
            assert_eq!(list.len(), 1);
        }

        #[test]
        fn a_moved_project_is_marked_missing_rather_than_dropped() {
            // The requirement is that the row *says* it is gone and offers
            // removal; silently pruning it would leave the user wondering whether
            // they had imagined the project.
            let mut list = list(&["/gone.musi", "/here.musi"]);
            list.probe(|path| path == Path::new("/here.musi"));
            assert_eq!(list.len(), 2);
            assert!(!list.entries()[0].missing, "/here.musi is present");
            assert!(list.entries()[1].missing, "/gone.musi is not");
            // And a probe is not sticky: a file that comes back stops being marked.
            list.probe(|_| true);
            assert!(list.entries().iter().all(|entry| !entry.missing));
        }

        #[test]
        fn a_list_round_trips_without_its_derived_flags() {
            let path = scratch("round-trip").join("nested/recent.json");
            let mut written = list(&["/a.musi", "/b.musi"]);
            written.probe(|_| false);
            save(&path, &written).unwrap();
            let read = load(&path).unwrap().expect("the file exists");
            // `missing` is derived, so it comes back false and is re-probed. The
            // stored halves are identical.
            assert!(read.entries().iter().all(|entry| !entry.missing));
            assert_eq!(read.len(), written.len());
            for (read, written) in read.entries().iter().zip(written.entries()) {
                assert_eq!(read.path, written.path);
                assert_eq!(read.name, written.name);
                assert_eq!(read.opened_unix, written.opened_unix);
            }
        }

        #[test]
        fn a_missing_file_is_a_new_user_and_not_a_fault() {
            let path = scratch("absent").join("recent.json");
            assert_eq!(load(&path), Ok(None));
        }

        #[test]
        fn a_corrupt_list_is_refused_and_left_byte_for_byte_on_disk() {
            // The whole point of the store: the application must keep running, and
            // must not overwrite something the user could still repair.
            let root = scratch("corrupt");
            let cases: [(&str, &[u8]); 5] = [
                ("truncated", b"{\"schema\":\"musializer.recent-pro"),
                (
                    "wrong-schema",
                    br#"{"schema":"musializer.recent-projects/v9","projects":[]}"#,
                ),
                (
                    "unknown-field",
                    br#"{"schema":"musializer.recent-projects/v1","projects":[],"extra":1}"#,
                ),
                (
                    "empty-path",
                    br#"{"schema":"musializer.recent-projects/v1","projects":[{"path":"","name":"x","opened_unix":null}]}"#,
                ),
                (
                    "duplicate-path",
                    br#"{"schema":"musializer.recent-projects/v1","projects":[
                        {"path":"/a.musi","name":"x","opened_unix":null},
                        {"path":"/a.musi","name":"y","opened_unix":null}]}"#,
                ),
            ];
            for (name, bytes) in cases {
                let path = root.join(format!("{name}.json"));
                std::fs::write(&path, bytes).unwrap();
                assert_eq!(
                    load(&path),
                    Err(RecentError::Format),
                    "{name} must be refused"
                );
                assert_eq!(std::fs::read(&path).unwrap(), bytes, "{name} must survive");
            }
        }

        #[test]
        fn an_over_long_list_is_refused_rather_than_truncated_on_read() {
            let path = scratch("over-capacity").join("recent.json");
            let rows: Vec<String> = (0..CAPACITY + 1)
                .map(|i| format!(r#"{{"path":"/p{i}.musi","name":"p","opened_unix":null}}"#))
                .collect();
            let bytes = format!(
                r#"{{"schema":"musializer.recent-projects/v1","projects":[{}]}}"#,
                rows.join(",")
            );
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(load(&path), Err(RecentError::Format));
        }

        #[test]
        fn an_entry_this_screen_could_not_draw_is_never_written() {
            let path = scratch("refuse-write").join("recent.json");
            for bad_name in ["", "   ", "two\nlines"] {
                let mut list = RecentProjects::default();
                list.record(PathBuf::from("/a.musi"), bad_name.to_string(), None);
                assert_eq!(save(&path, &list), Err(RecentError::Format), "{bad_name:?}");
            }
            let mut list = RecentProjects::default();
            list.record(PathBuf::from(""), "named".to_string(), None);
            assert_eq!(save(&path, &list), Err(RecentError::Format));
            assert!(!path.exists(), "a refused write must not create the file");
        }

        #[test]
        fn an_age_reads_as_words_and_never_as_the_future() {
            const NOW: i64 = 1_800_000_000;
            let at = |seconds_ago: i64| describe_age(Some(NOW - seconds_ago), Some(NOW));
            assert_eq!(at(0).as_deref(), Some("just now"));
            assert_eq!(at(59).as_deref(), Some("just now"));
            assert_eq!(at(60).as_deref(), Some("1 minute ago"));
            assert_eq!(at(59 * 60).as_deref(), Some("59 minutes ago"));
            assert_eq!(at(3600).as_deref(), Some("1 hour ago"));
            assert_eq!(at(23 * 3600).as_deref(), Some("23 hours ago"));
            assert_eq!(at(24 * 3600).as_deref(), Some("yesterday"));
            assert_eq!(at(47 * 3600).as_deref(), Some("yesterday"));
            assert_eq!(at(48 * 3600).as_deref(), Some("2 days ago"));
            assert_eq!(at(6 * 24 * 3600).as_deref(), Some("6 days ago"));
            assert_eq!(at(7 * 24 * 3600).as_deref(), Some("1 week ago"));
            assert_eq!(at(29 * 24 * 3600).as_deref(), Some("4 weeks ago"));
            assert_eq!(at(30 * 24 * 3600).as_deref(), Some("1 month ago"));
            assert_eq!(at(364 * 24 * 3600).as_deref(), Some("12 months ago"));
            assert_eq!(at(365 * 24 * 3600).as_deref(), Some("1 year ago"));
            assert_eq!(at(800 * 24 * 3600).as_deref(), Some("2 years ago"));
            // A clock that moved backwards, and an entry with no stamp at all.
            assert_eq!(at(-10_000).as_deref(), Some("just now"));
            assert_eq!(describe_age(None, Some(NOW)), None);
            assert_eq!(describe_age(Some(NOW), None), None);
        }
    }
}
