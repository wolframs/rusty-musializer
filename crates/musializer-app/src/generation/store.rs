//! Where generated clips, their jobs and the price list live on this machine.
//!
//! `$MUSIALIZER_GENERATED_DIR`, else `$XDG_DATA_HOME/musializer/generated-clips`,
//! else `~/.local/share/musializer/generated-clips`: data, not a cache — each
//! clip there was paid for, and a cache directory is one a cleaner may empty.
//! Saving a project bundles a placed clip like any other (its `.assets/clips/`);
//! the copy here is what lets a re-run of a treatment, or a second press of
//! Generate, find a clip it already has without asking for it again.
//!
//! ```text
//! clips/<request sha256>.mp4    the verified download
//! clips/<request sha256>.json   its provenance (model, prompt, cost, …)
//! jobs/<request sha256>.json    a job submitted and not yet downloaded
//! intents/<request sha256>.json a request about to be sent, or sent and
//!                               never answered: it may have been billed
//! locks/<request sha256>.lock   held by the one caller asking for a request
//! work/<name>/                  one caller's scratch (body, answers, partial)
//! prices.json                   the price list, with when it was read
//! spending.tsv                  one line per job: when, what, estimate, cost
//! ```
//!
//! The folder is created `0700` and every file `0600`: the request body is a
//! prompt and frames, not a secret, but nobody else on the machine needs it.
//!
//! **One owner per request** (H4b, 2026-11-14). Two callers — two windows, a
//! window and a treatment run — can ask for the same request at once. From
//! "is there a clip or a job for it already" until "its job id is on disk",
//! the caller holds the request's lock ([`Store::claim`]); a second caller
//! waits on it and then finds the job the first recorded, instead of paying
//! for it again. The lock is given back with `File::unlock` before the file
//! closes (`docs/TRAPS.md`, "A lock given back by closing its file").

use std::fs::{File, TryLockError};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use musializer_core::generation::catalog::Catalog;
use musializer_core::project::clip::ClipProvenance;
use musializer_core::project::sha256;
use serde::{Deserialize, Serialize};

const JOB_SCHEMA: &str = "musializer.video-job/v1";
const INTENT_SCHEMA: &str = "musializer.video-intent/v1";
const RECEIPT_SCHEMA: &str = "musializer.video-clip/v1";
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

/// The lock on one request, held by the caller that may look it up and send
/// it. Given back by [`Claim::release`] or on drop, explicitly unlocked first.
#[derive(Debug)]
pub struct Claim {
    file: Option<File>,
}

impl Claim {
    /// Gives the request back to the next caller.
    pub fn release(mut self) {
        self.unlock();
    }

    fn unlock(&mut self) {
        if let Some(file) = self.file.take() {
            // Before the close: a child another thread is spawning holds a
            // copy of the open file until it execs.
            let _ = file.unlock();
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.unlock();
    }
}

/// Scratch folders of this process, so two callers never share one.
static WORK_SERIAL: AtomicU64 = AtomicU64::new(0);

/// A job that was submitted: enough to poll it again after a cancel, a crash
/// or a restart, instead of paying for it twice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRecord {
    pub schema: String,
    pub request_sha256: String,
    pub job_id: String,
    pub model: String,
    pub seconds: u32,
    pub submitted_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_usd: Option<f64>,
}

/// A request written down before it is sent (H4b). It becomes the job's
/// record when the job id arrives, and goes when the provider refused it or
/// it never left. One that is still here with nobody sending it means: a
/// request may have been accepted, and its job id is unknown. OpenRouter has
/// no way to find such a job (`docs/VIDEO_GENERATION.md`, read 2026-11-14), so
/// the next ask for it says so and waits for the person's agreement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub schema: String,
    pub request_sha256: String,
    pub model: String,
    pub seconds: u32,
    /// When it was sent, UTC.
    pub sent_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_usd: Option<f64>,
}

/// The provenance beside a stored clip, as plain JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    provider: String,
    model: String,
    prompt: String,
    seconds: u32,
    resolution: String,
    aspect_ratio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_sha256: Option<String>,
    request_sha256: String,
    generated_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    estimate_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    /// The digest of the clip's bytes, so a file swapped in the store is
    /// not mistaken for the clip that was paid for.
    clip_sha256: String,
}

fn is_digest(text: &str) -> bool {
    sha256::is_hex_digest(text)
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_RECORD_BYTES).then_some(bytes)
}

impl Store {
    /// The folder the environment names.
    ///
    /// # Errors
    ///
    /// No `MUSIALIZER_GENERATED_DIR`, `XDG_DATA_HOME` or `HOME`.
    pub fn from_env() -> Result<Self, String> {
        let root = std::env::var_os("MUSIALIZER_GENERATED_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_DATA_HOME")
                    .filter(|value| !value.is_empty())
                    .map(|data| PathBuf::from(data).join("musializer/generated-clips"))
            })
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".local/share/musializer/generated-clips"))
            })
            .ok_or("no folder for generated clips: set HOME or XDG_DATA_HOME")?;
        Ok(Self { root })
    }

    #[cfg(test)]
    #[must_use]
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The store's folder `name`, made `0700` if it is not there. A folder
    /// made now is synced into its parent (and the store's root into its
    /// own), so an intent written into a fresh `intents/` survives a power
    /// loss with the folder that holds it.
    fn folder(&self, name: &str) -> Result<PathBuf, String> {
        let folder = self.root.join(name);
        let made_root = !self.root.is_dir();
        let made = !folder.is_dir();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
        let mut synced = Vec::new();
        if made_root {
            synced.extend(self.root.parent().map(Path::to_path_buf));
        }
        if made {
            synced.extend(folder.parent().map(Path::to_path_buf));
        }
        for parent in synced {
            sync_dir(&parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        Ok(folder)
    }

    /// A private, empty scratch folder of this caller's own, named after
    /// `what` and this process: two callers (two windows, a window and a
    /// treatment run) never empty each other's. Returns its name, for
    /// [`Self::clear_work`], and its path.
    ///
    /// # Errors
    ///
    /// The folder cannot be made.
    pub fn private_work(&self, what: &str) -> Result<(String, PathBuf), String> {
        let name = format!(
            "{what}p{}n{}",
            std::process::id(),
            WORK_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let path = self.work(&name)?;
        Ok((name, path))
    }

    /// A private, empty scratch folder named `request`.
    ///
    /// # Errors
    ///
    /// The folder cannot be made.
    pub fn work(&self, request: &str) -> Result<PathBuf, String> {
        let folder = self.folder("work")?.join(request);
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&folder)
            .map_err(|error| format!("{}: {error}", folder.display()))?;
        Ok(folder)
    }

    /// Removes a job's scratch.
    pub fn clear_work(&self, request: &str) {
        if !request.is_empty() && request.chars().all(|c| c.is_ascii_alphanumeric()) {
            let _ = std::fs::remove_dir_all(self.root.join("work").join(request));
        }
    }

    /// Where the clip for a request is kept.
    #[must_use]
    pub fn clip_path(&self, request: &str) -> PathBuf {
        self.root.join("clips").join(format!("{request}.mp4"))
    }

    /// The clip this store has for a request and its provenance, when it has
    /// one whose bytes are the ones its receipt names.
    #[must_use]
    pub fn find(&self, request: &str) -> Option<(PathBuf, ClipProvenance)> {
        if !is_digest(request) {
            return None;
        }
        let path = self.clip_path(request);
        let receipt: Receipt = serde_json::from_slice(&read_bounded(
            &self.root.join("clips").join(format!("{request}.json")),
        )?)
        .ok()?;
        if receipt.schema != RECEIPT_SCHEMA || receipt.request_sha256 != request {
            return None;
        }
        let digest = musializer_runtime::project_files::sha256_file_hex(&path).ok()?;
        (digest == receipt.clip_sha256).then_some((
            path,
            ClipProvenance {
                provider: receipt.provider,
                model: receipt.model,
                prompt: receipt.prompt,
                seconds: receipt.seconds,
                resolution: receipt.resolution,
                aspect_ratio: receipt.aspect_ratio,
                first_sha256: receipt.first_sha256,
                last_sha256: receipt.last_sha256,
                request_sha256: receipt.request_sha256,
                generated_utc: receipt.generated_utc,
                estimate_usd: receipt.estimate_usd,
                cost_usd: receipt.cost_usd,
            },
        ))
    }

    /// Moves a verified download in and writes its receipt.
    ///
    /// # Errors
    ///
    /// The folder or a file cannot be written.
    pub fn keep(
        &self,
        download: &Path,
        record: &ClipProvenance,
        clip_sha256: &str,
    ) -> Result<PathBuf, String> {
        let clips = self.folder("clips")?;
        let destination = self.clip_path(&record.request_sha256);
        // curl created the download under the umask; the store's files are
        // the person's alone.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(download, std::fs::Permissions::from_mode(0o600));
        // curl wrote it and never synced it: a paid clip reaches the disk
        // before its name moves into the store, and the name with the folder.
        sync_file(download).map_err(|error| format!("{}: {error}", download.display()))?;
        std::fs::rename(download, &destination)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
        sync_dir(&clips).map_err(|error| format!("{}: {error}", clips.display()))?;
        let receipt = Receipt {
            schema: RECEIPT_SCHEMA.to_owned(),
            provider: record.provider.clone(),
            model: record.model.clone(),
            prompt: record.prompt.clone(),
            seconds: record.seconds,
            resolution: record.resolution.clone(),
            aspect_ratio: record.aspect_ratio.clone(),
            first_sha256: record.first_sha256.clone(),
            last_sha256: record.last_sha256.clone(),
            request_sha256: record.request_sha256.clone(),
            generated_utc: record.generated_utc.clone(),
            estimate_usd: record.estimate_usd,
            cost_usd: record.cost_usd,
            clip_sha256: clip_sha256.to_owned(),
        };
        write_private(
            &clips.join(format!("{}.json", record.request_sha256)),
            serde_json::to_string_pretty(&receipt)
                .map_err(|error| error.to_string())?
                .as_bytes(),
        )?;
        Ok(destination)
    }

    /// The job submitted for a request and not yet finished, if any.
    #[must_use]
    pub fn job(&self, request: &str) -> Option<JobRecord> {
        if !is_digest(request) {
            return None;
        }
        let record: JobRecord = serde_json::from_slice(&read_bounded(
            &self.root.join("jobs").join(format!("{request}.json")),
        )?)
        .ok()?;
        (record.schema == JOB_SCHEMA && record.request_sha256 == request).then_some(record)
    }

    /// Records a submitted job, before anything else happens to it.
    ///
    /// # Errors
    ///
    /// The folder or file cannot be written.
    pub fn record_job(&self, record: &JobRecord) -> Result<(), String> {
        let jobs = self.folder("jobs")?;
        let mut record = record.clone();
        record.schema = JOB_SCHEMA.to_owned();
        write_private(
            &jobs.join(format!("{}.json", record.request_sha256)),
            serde_json::to_string_pretty(&record)
                .map_err(|error| error.to_string())?
                .as_bytes(),
        )
    }

    /// The intent written for a request, if any.
    #[must_use]
    pub fn intent(&self, request: &str) -> Option<Intent> {
        if !is_digest(request) {
            return None;
        }
        let intent: Intent = serde_json::from_slice(&read_bounded(
            &self.root.join("intents").join(format!("{request}.json")),
        )?)
        .ok()?;
        (intent.schema == INTENT_SCHEMA && intent.request_sha256 == request).then_some(intent)
    }

    /// An intent nobody is sending now: a request that may have been
    /// accepted and whose job id is unknown.
    #[must_use]
    pub fn leftover_intent(&self, request: &str) -> Option<Intent> {
        if self.in_flight(request) {
            return None;
        }
        self.intent(request)
    }

    /// Writes the intent, before the request leaves.
    ///
    /// # Errors
    ///
    /// The folder or file cannot be written.
    pub fn record_intent(&self, intent: &Intent) -> Result<(), String> {
        let intents = self.folder("intents")?;
        let mut intent = intent.clone();
        intent.schema = INTENT_SCHEMA.to_owned();
        write_private(
            &intents.join(format!("{}.json", intent.request_sha256)),
            serde_json::to_string_pretty(&intent)
                .map_err(|error| error.to_string())?
                .as_bytes(),
        )
    }

    pub fn forget_intent(&self, request: &str) {
        if is_digest(request) {
            let _ = std::fs::remove_file(self.root.join("intents").join(format!("{request}.json")));
        }
    }

    pub fn forget_job(&self, request: &str) {
        if is_digest(request) {
            let _ = std::fs::remove_file(self.root.join("jobs").join(format!("{request}.json")));
        }
    }

    fn lock_file(&self, request: &str) -> Result<File, String> {
        if !is_digest(request) {
            return Err(format!("{request} is not a request digest"));
        }
        let path = self.folder("locks")?.join(format!("{request}.lock"));
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Takes the request's lock, waiting while another caller holds it and
    /// calling `waiting` meanwhile. `Ok(None)` when `stop` was set first.
    ///
    /// # Errors
    ///
    /// The lock file cannot be opened or locked.
    pub fn claim(
        &self,
        request: &str,
        stop: &AtomicBool,
        mut waiting: impl FnMut(),
    ) -> Result<Option<Claim>, String> {
        let file = self.lock_file(request)?;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(Claim { file: Some(file) })),
                Err(TryLockError::WouldBlock) => {
                    waiting();
                    if stop.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(TryLockError::Error(error)) => {
                    return Err(format!("the lock on request {request}: {error}"))
                }
            }
        }
    }

    /// Another caller is sending this request now (it holds the lock).
    #[must_use]
    pub fn in_flight(&self, request: &str) -> bool {
        let Ok(file) = self.lock_file(request) else {
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(TryLockError::WouldBlock) => true,
            Err(TryLockError::Error(_)) => false,
        }
    }

    /// The cached price list, if it reads.
    #[must_use]
    pub fn prices(&self) -> Option<Catalog> {
        Catalog::from_cache(&read_bounded_large(&self.root.join("prices.json"))?).ok()
    }

    /// Replaces the cached price list.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn keep_prices(&self, catalog: &Catalog) -> Result<(), String> {
        self.folder("")?;
        write_private(
            &self.root.join("prices.json"),
            catalog.to_cache().as_bytes(),
        )
    }

    /// Appends one line to `spending.tsv`.
    pub fn log_spending(&self, line: &[&str]) {
        if self.folder("").is_err() {
            return;
        }
        let path = self.root.join("spending.tsv");
        let fresh = !path.exists();
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
        else {
            return;
        };
        if fresh {
            let _ = writeln!(
                file,
                "utc\trequest_sha256\tjob_id\tmodel\tseconds\testimate_usd\tcost_usd\toutcome"
            );
        }
        let cells: Vec<String> = line
            .iter()
            .map(|cell| cell.replace(['\t', '\n', '\r'], " "))
            .collect();
        let _ = writeln!(file, "{}", cells.join("\t"));
        // The person's record of what may have cost money: on the disk with
        // the line, and a new file with its folder.
        let _ = file.sync_data();
        if fresh {
            let _ = sync_dir(&self.root);
        }
    }
}

fn read_bounded_large(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    let cap = musializer_core::generation::catalog::MAX_BYTES as u64;
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

#[cfg(test)]
thread_local! {
    /// Every folder and file this thread synced, for the tests: a power loss
    /// cannot be staged in a unit test, but whether the folder was synced
    /// after the rename can be seen.
    static SYNCED: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `fsync` on a folder: a rename into it, or a file made in it, is durable
/// only once the folder's own entry list is on the disk.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    SYNCED.with(|synced| synced.borrow_mut().push(dir.to_path_buf()));
    File::open(dir)?.sync_all()
}

/// `fsync` on a file someone else wrote (curl's download).
fn sync_file(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    SYNCED.with(|synced| synced.borrow_mut().push(path.to_path_buf()));
    std::fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Writes a file `0600` through a sibling and a rename, durably: the bytes
/// are synced before the rename and the folder after it, so a power loss
/// right after the provider accepted a request keeps the intent or the job
/// that says so (H4c).
///
/// # Errors
///
/// The write, the rename or a sync fails.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("partial");
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        sync_dir(path.parent().unwrap_or(Path::new(".")))
    })();
    result.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {error}", path.display())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh folder under the system's temporary directory, removed on drop.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "mz-generation-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record(request: &str) -> ClipProvenance {
        ClipProvenance {
            provider: "openrouter".into(),
            model: "google/veo-3.1-lite".into(),
            prompt: "fog".into(),
            seconds: 4,
            resolution: "720p".into(),
            aspect_ratio: "16:9".into(),
            first_sha256: None,
            last_sha256: None,
            request_sha256: request.into(),
            generated_utc: "2026-11-14T20:00:00Z".into(),
            estimate_usd: Some(0.12),
            cost_usd: Some(0.12),
        }
    }

    #[test]
    fn a_kept_clip_is_found_by_its_request_and_only_while_its_bytes_are_the_same() {
        let folder = Scratch::new("keep");
        let store = Store::at(folder.path().join("store"));
        let request = sha256::digest_hex(b"request");
        assert!(store.find(&request).is_none());
        let download = folder.path().join("clip.part");
        std::fs::write(&download, b"video bytes").unwrap();
        let kept = store
            .keep(
                &download,
                &record(&request),
                &sha256::digest_hex(b"video bytes"),
            )
            .unwrap();
        let (path, found) = store.find(&request).unwrap();
        assert_eq!(path, kept);
        assert_eq!(found, record(&request));
        std::fs::write(&kept, b"other bytes").unwrap();
        assert!(
            store.find(&request).is_none(),
            "a swapped file is not the clip"
        );
        assert!(store.find("../../etc").is_none());
    }

    fn synced() -> Vec<PathBuf> {
        SYNCED.with(|synced| std::mem::take(&mut *synced.borrow_mut()))
    }

    /// What a test can show of durability: every record's folder is opened
    /// and synced after the rename, a folder made for it is synced into its
    /// parent, and a kept clip's bytes are synced before their name moves.
    /// Whether the disk then keeps them through a power loss is the kernel's
    /// and cannot be staged here.
    #[test]
    fn every_record_is_synced_with_its_folder() {
        let folder = Scratch::new("durable");
        let root = folder.path().join("store");
        let store = Store::at(root.clone());
        let request = sha256::digest_hex(b"durable");
        synced();
        store
            .record_intent(&Intent {
                schema: String::new(),
                request_sha256: request.clone(),
                model: "m/x".into(),
                seconds: 4,
                sent_utc: "2026-11-14T20:00:00Z".into(),
                estimate_usd: Some(0.12),
            })
            .unwrap();
        let seen = synced();
        // The fresh root into its parent, `intents/` into the root, the
        // intent into `intents/`.
        assert_eq!(
            seen,
            vec![folder.path().to_path_buf(), root.clone(), root.join("intents")],
            "{seen:?}"
        );
        store
            .record_job(&JobRecord {
                schema: String::new(),
                request_sha256: request.clone(),
                job_id: "gen-vid-1-abc".into(),
                model: "m/x".into(),
                seconds: 4,
                submitted_utc: "2026-11-14T20:00:00Z".into(),
                estimate_usd: Some(0.12),
            })
            .unwrap();
        assert_eq!(synced(), vec![root.clone(), root.join("jobs")]);
        // A second record in an existing folder syncs only that folder.
        store
            .record_job(&JobRecord {
                schema: String::new(),
                request_sha256: request.clone(),
                job_id: "gen-vid-2-abc".into(),
                model: "m/x".into(),
                seconds: 4,
                submitted_utc: "2026-11-14T20:00:01Z".into(),
                estimate_usd: Some(0.12),
            })
            .unwrap();
        assert_eq!(synced(), vec![root.join("jobs")]);
        let download = folder.path().join("clip.part");
        std::fs::write(&download, b"video bytes").unwrap();
        store
            .keep(
                &download,
                &record(&request),
                &sha256::digest_hex(b"video bytes"),
            )
            .unwrap();
        let clips = root.join("clips");
        assert_eq!(
            synced(),
            vec![root.clone(), download, clips.clone(), clips.clone()],
            "the clip's bytes, then its name, then its receipt's"
        );
        store.log_spending(&["a", "b"]);
        assert_eq!(synced(), vec![root.clone()], "a new ledger with its folder");
    }

    #[test]
    fn a_job_record_survives_until_it_is_forgotten() {
        let folder = Scratch::new("job");
        let store = Store::at(folder.path().to_path_buf());
        let request = sha256::digest_hex(b"r");
        let job = JobRecord {
            schema: String::new(),
            request_sha256: request.clone(),
            job_id: "gen-vid-1-abc".into(),
            model: "m/x".into(),
            seconds: 4,
            submitted_utc: "2026-11-14T20:00:00Z".into(),
            estimate_usd: Some(0.2),
        };
        store.record_job(&job).unwrap();
        let read = store.job(&request).unwrap();
        assert_eq!(read.job_id, "gen-vid-1-abc");
        assert_eq!(read.schema, JOB_SCHEMA);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(folder.path().join("jobs").join(format!("{request}.json")))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        store.forget_job(&request);
        assert!(store.job(&request).is_none());
    }
}
