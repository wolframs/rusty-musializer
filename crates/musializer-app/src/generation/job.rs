//! One generation, on its own thread: submit, poll, download, verify.
//!
//! The window never waits on it: it reads [`GenerationJob::progress`] each
//! frame and takes the [`Outcome`] once. The command line waits on it with
//! [`GenerationJob::wait`]. Either way:
//!
//! - **Nothing is paid for twice.** A clip the store already has for the
//!   request is the outcome at once, with no call; a job submitted earlier
//!   and never downloaded (cancelled, failed to download, the application
//!   closed) is polled again instead of submitted again.
//! - **Nothing is half-imported.** The download goes to the job's scratch
//!   folder, is opened by its handle and probed as the clip importer does
//!   (`clips::import`), and only then moves into the store. A download that
//!   is not a video, is cut short, or is larger than allowed is removed and
//!   named.
//! - **A stop is honest about money.** OpenRouter has no call that cancels a
//!   job (2026-11-14), so a stop after the submit stops *waiting*; the job may
//!   still finish and be billed, and the outcome says so with the estimate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use musializer_core::assist::secret::Secret;
use musializer_core::generation::request::GenerationRequest;
use musializer_core::project::clip::ClipProvenance;

use super::client::{Client, ClientError, Endpoint, JobState};
use super::store::{JobRecord, Store};
use crate::clips::TrackClip;

/// The largest download a clip may be. A 30 s 4K clip from the picks is a few
/// hundred MB; the clip importer's own cap (4 GiB) is for files a person
/// chose. `MUSIALIZER_VIDEO_MAX_BYTES` may lower it (the gate does), never
/// raise it.
pub const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

/// How often a job is asked about: OpenRouter suggests every 10 to 30 s.
pub const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How long a job may take before this application stops waiting for it.
pub const DEADLINE: Duration = Duration::from_secs(30 * 60);

/// What a job is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Starting,
    /// Sending the request.
    Submitting,
    /// Submitted; the provider's word for where it is.
    Waiting {
        status: String,
    },
    Downloading,
    Verifying,
}

impl Stage {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Starting => "starting".into(),
            Self::Submitting => "sending the request".into(),
            Self::Waiting { status } => match status.as_str() {
                "pending" => "queued at the provider".into(),
                "in_progress" => "the model is making it".into(),
                other => format!("waiting ({other})"),
            },
            Self::Downloading => "downloading".into(),
            Self::Verifying => "checking the download".into(),
        }
    }
}

/// What a finished, failed or stopped job may have cost.
#[derive(Clone, Debug, PartialEq)]
pub enum Billing {
    /// Nothing was submitted.
    Nothing,
    /// The provider reported a cost.
    Reported(f64),
    /// Submitted and not reported: it may be billed, at about the estimate.
    Possibly(Option<f64>),
}

impl Billing {
    #[must_use]
    pub fn sentence(&self) -> Option<String> {
        match self {
            Self::Nothing => None,
            Self::Reported(usd) => Some(format!("OpenRouter reported ${usd:.2} for it.")),
            Self::Possibly(Some(usd)) => Some(format!(
                "It was already submitted, so it may still be billed (about ${:.2}).",
                musializer_core::generation::price::ceil_cents(*usd)
            )),
            Self::Possibly(None) => {
                Some("It was already submitted, so it may still be billed (price unknown).".into())
            }
        }
    }
}

/// A clip that is ready to import.
#[derive(Clone, Debug)]
pub struct Generated {
    pub clip: TrackClip,
    /// Found in the store or polled to completion from an earlier submit,
    /// rather than submitted now.
    pub reused: Reuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reuse {
    /// Submitted, generated and downloaded now.
    New,
    /// The store had it; nothing was sent.
    Stored,
    /// An earlier submit was picked up; nothing new was paid for.
    Resumed,
}

/// How a job ended.
#[derive(Clone, Debug)]
pub enum Outcome {
    Done(Generated),
    Failed { reason: String, billing: Billing },
    Stopped { billing: Billing },
}

/// Everything a job needs, owned.
pub struct JobSpec {
    pub request: GenerationRequest,
    /// The frame images' bytes, as rendered (PNG); `request.first`/`last`
    /// carry their digests.
    pub first_png: Option<Vec<u8>>,
    pub last_png: Option<Vec<u8>>,
    pub estimate_usd: Option<f64>,
    /// The job's own copy, dropped (and zeroized) with the job.
    pub secret: Secret,
    pub endpoint: Endpoint,
    pub store: Store,
    pub poll: Duration,
    pub deadline: Duration,
    pub max_bytes: u64,
    /// A name for the clip in the library.
    pub name: String,
}

impl JobSpec {
    /// The timings the environment sets for a gate (`MUSIALIZER_VIDEO_POLL_MS`,
    /// `MUSIALIZER_VIDEO_DEADLINE_S`, `MUSIALIZER_VIDEO_MAX_BYTES`), else the
    /// defaults.
    #[must_use]
    pub fn timings_from_env() -> (Duration, Duration, u64) {
        let number = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
        };
        (
            number("MUSIALIZER_VIDEO_POLL_MS")
                .map_or(POLL_INTERVAL, |ms| Duration::from_millis(ms.max(20))),
            number("MUSIALIZER_VIDEO_DEADLINE_S")
                .map_or(DEADLINE, |seconds| Duration::from_secs(seconds.max(1))),
            number("MUSIALIZER_VIDEO_MAX_BYTES")
                .map_or(MAX_DOWNLOAD_BYTES, |bytes| bytes.min(MAX_DOWNLOAD_BYTES)),
        )
    }
}

struct Shared {
    stage: Stage,
    job_id: Option<String>,
    outcome: Option<Outcome>,
}

/// A running generation.
pub struct GenerationJob {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    handle: Option<JoinHandle<()>>,
    started: Instant,
}

impl GenerationJob {
    #[must_use]
    pub fn spawn(spec: JobSpec) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Shared {
            stage: Stage::Starting,
            job_id: None,
            outcome: None,
        }));
        let handle = {
            let stop = Arc::clone(&stop);
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("video-generation".into())
                .spawn(move || {
                    let outcome = run(&spec, &stop, &shared);
                    if let Ok(mut shared) = shared.lock() {
                        shared.outcome = Some(outcome);
                    }
                })
                .ok()
        };
        if handle.is_none() {
            if let Ok(mut shared) = shared.lock() {
                shared.outcome = Some(Outcome::Failed {
                    reason: "the job's thread could not be started".into(),
                    billing: Billing::Nothing,
                });
            }
        }
        Self {
            stop,
            shared,
            handle,
            started: Instant::now(),
        }
    }

    /// What it is doing, and for how long it has been running.
    #[must_use]
    pub fn progress(&self) -> (Stage, Duration) {
        let stage = self
            .shared
            .lock()
            .map_or(Stage::Starting, |shared| shared.stage.clone());
        (stage, self.started.elapsed())
    }

    /// The provider's id for the job, once submitted.
    #[must_use]
    pub fn job_id(&self) -> Option<String> {
        self.shared
            .lock()
            .ok()
            .and_then(|shared| shared.job_id.clone())
    }

    /// The outcome, once, when it has one.
    pub fn take_outcome(&mut self) -> Option<Outcome> {
        let outcome = self.shared.lock().ok()?.outcome.take();
        if outcome.is_some() {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
        outcome
    }

    /// Stops waiting; the outcome follows within a fraction of a second.
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Blocks until it ends, calling `report` whenever the stage changes.
    pub fn wait(mut self, mut report: impl FnMut(&Stage, Duration)) -> Outcome {
        let mut last = None;
        loop {
            if let Some(outcome) = self.take_outcome() {
                return outcome;
            }
            let (stage, elapsed) = self.progress();
            if last.as_ref() != Some(&stage) {
                report(&stage, elapsed);
                last = Some(stage);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for GenerationJob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn set_stage(shared: &Mutex<Shared>, stage: Stage) {
    if let Ok(mut shared) = shared.lock() {
        shared.stage = stage;
    }
}

fn now_utc() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    musializer_core::generation::catalog::utc_timestamp(now)
}

fn money(value: Option<f64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |usd| format!("{usd:.4}"))
}

/// Sleeps `total`, waking for a stop.
fn pause(total: Duration, stop: &AtomicBool) -> bool {
    let until = Instant::now() + total;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20).min(total));
    }
    !stop.load(Ordering::Relaxed)
}

fn run(spec: &JobSpec, stop: &AtomicBool, shared: &Mutex<Shared>) -> Outcome {
    let digest = spec.request.digest();
    let store = &spec.store;

    // 1. The store has it: nothing is sent.
    if let Some((path, record)) = store.find(&digest) {
        return match import(&path, record, &spec.name) {
            Ok(clip) => Outcome::Done(Generated {
                clip,
                reused: Reuse::Stored,
            }),
            Err(reason) => Outcome::Failed {
                reason: format!("the stored clip for this request no longer reads: {reason}"),
                billing: Billing::Nothing,
            },
        };
    }

    let work = match store.work(&digest) {
        Ok(work) => work,
        Err(reason) => {
            return Outcome::Failed {
                reason,
                billing: Billing::Nothing,
            }
        }
    };
    let client = Client {
        endpoint: &spec.endpoint,
        secret: &spec.secret,
        work: &work,
        stop,
    };
    let outcome = run_with(spec, &client, &digest, stop, shared);
    store.clear_work(&digest);
    let (verdict, cost) = match &outcome {
        Outcome::Done(generated) => (
            match generated.reused {
                Reuse::New => "generated",
                Reuse::Stored => "stored",
                Reuse::Resumed => "resumed",
            }
            .to_owned(),
            generated
                .clip
                .asset
                .generated
                .as_ref()
                .and_then(|record| record.cost_usd),
        ),
        Outcome::Failed { billing, .. } => (
            "failed".to_owned(),
            match billing {
                Billing::Reported(usd) => Some(*usd),
                _ => None,
            },
        ),
        Outcome::Stopped { .. } => ("stopped".to_owned(), None),
    };
    if !matches!(
        &outcome,
        Outcome::Failed {
            billing: Billing::Nothing,
            ..
        }
    ) && !matches!(
        &outcome,
        Outcome::Stopped {
            billing: Billing::Nothing
        }
    ) {
        let job_id = shared
            .lock()
            .ok()
            .and_then(|shared| shared.job_id.clone())
            .unwrap_or_else(|| "-".to_owned());
        store.log_spending(&[
            &now_utc(),
            &digest,
            &job_id,
            &spec.request.model,
            &spec.request.seconds.to_string(),
            &money(spec.estimate_usd),
            &money(cost),
            &verdict,
        ]);
    }
    outcome
}

fn run_with(
    spec: &JobSpec,
    client: &Client<'_>,
    digest: &str,
    stop: &AtomicBool,
    shared: &Mutex<Shared>,
) -> Outcome {
    let store = &spec.store;
    let possibly = Billing::Possibly(spec.estimate_usd);

    // 2. An earlier submit for the same request is picked up, not repeated.
    let (job_id, resumed) = if let Some(record) = store.job(digest) {
        (record.job_id, true)
    } else {
        if stop.load(Ordering::Relaxed) {
            return Outcome::Stopped {
                billing: Billing::Nothing,
            };
        }
        set_stage(shared, Stage::Submitting);
        let urls = match (
            spec.first_png
                .as_deref()
                .map(super::client::data_url)
                .transpose(),
            spec.last_png
                .as_deref()
                .map(super::client::data_url)
                .transpose(),
        ) {
            (Ok(first), Ok(last)) => (first, last),
            (Err(reason), _) | (_, Err(reason)) => {
                return Outcome::Failed {
                    reason,
                    billing: Billing::Nothing,
                }
            }
        };
        let body = spec.request.body(urls.0.as_deref(), urls.1.as_deref());
        let body_path = client.work.join("request.json");
        if let Err(reason) = super::store::write_private(&body_path, body.to_string().as_bytes()) {
            return Outcome::Failed {
                reason,
                billing: Billing::Nothing,
            };
        }
        let state = match client.submit(&body_path) {
            Ok(state) => state,
            Err(ClientError::Stopped) => {
                // The request may have reached OpenRouter before the stop.
                return Outcome::Stopped { billing: possibly };
            }
            Err(error) => {
                return Outcome::Failed {
                    reason: error.to_string(),
                    // A refused submit created no job.
                    billing: Billing::Nothing,
                };
            }
        };
        let record = JobRecord {
            schema: String::new(),
            request_sha256: digest.to_owned(),
            job_id: state.id.clone(),
            model: spec.request.model.clone(),
            seconds: spec.request.seconds,
            submitted_utc: now_utc(),
            estimate_usd: spec.estimate_usd,
        };
        if let Err(reason) = store.record_job(&record) {
            // Without the record a stop would lose the job: say so, keep going.
            eprintln!("warning: the job could not be recorded: {reason}");
        }
        (state.id, false)
    };
    if let Ok(mut shared) = shared.lock() {
        shared.job_id = Some(job_id.clone());
    }

    // 3. Poll until it ends.
    let started = Instant::now();
    let mut first = !resumed;
    let state: JobState = loop {
        if !first && !pause(spec.poll, stop) {
            return Outcome::Stopped { billing: possibly };
        }
        first = false;
        match client.poll(&job_id) {
            Ok(state) if state.is_terminal() => break state,
            Ok(state) => set_stage(
                shared,
                Stage::Waiting {
                    status: state.status,
                },
            ),
            Err(ClientError::Stopped) => return Outcome::Stopped { billing: possibly },
            Err(ClientError::Http(refusal)) if refusal.status == 404 => {
                store.forget_job(digest);
                return Outcome::Failed {
                    reason: "OpenRouter no longer knows the job submitted for this request; \
                             Generate again asks (and pays) anew"
                        .into(),
                    billing: possibly,
                };
            }
            // A poll that fails is asked again until the deadline: the job
            // is not lost because one answer was.
            Err(error) => {
                if started.elapsed() > spec.deadline {
                    return Outcome::Failed {
                        reason: format!("gave up asking about the job: {error}"),
                        billing: possibly,
                    };
                }
            }
        }
        if started.elapsed() > spec.deadline {
            return Outcome::Failed {
                reason: format!(
                    "the job took longer than {} minutes; it may still finish, and Generate \
                     again picks it up",
                    spec.deadline.as_secs() / 60
                ),
                billing: possibly,
            };
        }
    };
    let billing = state.cost_usd.map_or(possibly.clone(), Billing::Reported);
    if state.status != "completed" {
        store.forget_job(digest);
        return Outcome::Failed {
            reason: format!(
                "the job {}{}",
                state.status,
                state
                    .error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ),
            billing,
        };
    }

    // 4. Download, verify, keep.
    set_stage(shared, Stage::Downloading);
    let part = client.work.join("clip.part");
    if let Err(error) = client.download(&job_id, &part, spec.max_bytes) {
        let _ = std::fs::remove_file(&part);
        return match error {
            ClientError::Stopped => Outcome::Stopped { billing },
            other => Outcome::Failed {
                reason: format!("the clip could not be downloaded: {other}"),
                billing,
            },
        };
    }
    set_stage(shared, Stage::Verifying);
    let record = spec
        .request
        .provenance(&now_utc(), spec.estimate_usd, state.cost_usd);
    let verified = import(&part, record.clone(), &spec.name);
    let clip = match verified {
        Ok(clip) => clip,
        Err(reason) => {
            let _ = std::fs::remove_file(&part);
            store.forget_job(digest);
            return Outcome::Failed {
                reason: format!("the download is not a clip this application can play: {reason}"),
                billing,
            };
        }
    };
    let kept = match store.keep(&part, &record, &clip.asset.sha256) {
        Ok(kept) => kept,
        Err(reason) => {
            return Outcome::Failed {
                reason: format!("the clip could not be kept: {reason}"),
                billing,
            }
        }
    };
    store.forget_job(digest);
    Outcome::Done(Generated {
        clip: TrackClip {
            source: kept,
            ..clip
        },
        reused: if resumed { Reuse::Resumed } else { Reuse::New },
    })
}

/// Reads a downloaded or stored clip the way "Import clip…" does, and gives
/// it its provenance and name.
fn import(path: &std::path::Path, record: ClipProvenance, name: &str) -> Result<TrackClip, String> {
    let mut clip = crate::clips::import(path).map_err(|error| error.to_string())?;
    clip.asset.name = name.to_owned();
    clip.asset.generated = Some(record);
    Ok(clip)
}

/// The library name of a generated clip: the prompt's start and the model,
/// bounded like any clip name.
#[must_use]
pub fn clip_name(request: &GenerationRequest) -> String {
    let words: String = request
        .prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect();
    let label = musializer_core::generation::pick(&request.model)
        .map_or(request.model.as_str(), |pick| pick.label);
    let mut name = format!("{words} ({label}).mp4");
    while name.len() > musializer_core::project::clip::CLIP_NAME_MAX_BYTES {
        name.remove(0);
    }
    name
}

/// Where a generated clip came from, for a notice.
#[must_use]
pub fn kept_at(store: &Store) -> PathBuf {
    store.root().join("clips")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;

    use musializer_core::generation::price::Shape;
    use musializer_core::project::sha256;

    use super::super::store::tests::Scratch;
    use super::*;

    /// What the fake provider does with one job.
    #[derive(Clone)]
    pub(crate) struct Script {
        /// The submit's status and body, or `None` for a normal 202.
        pub submit: Option<(u16, String)>,
        /// Polls answered `in_progress` before the end.
        pub polls: usize,
        /// The final poll's body (its `status`, `error`, `usage`).
        pub end: serde_json::Value,
        /// The download's content type and bytes.
        pub content: (String, Vec<u8>),
    }

    /// A tiny HTTP/1.1 server on loopback speaking the four calls.
    pub(crate) struct Fake {
        pub base: String,
        pub counts: Arc<Mutex<HashMap<String, usize>>>,
        pub keys: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Fake {
        pub(crate) fn start(script: Script) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!(
                "http://127.0.0.1:{}/api/v1",
                listener.local_addr().unwrap().port()
            );
            let counts: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
            let keys: Arc<Mutex<Vec<String>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let polls = Arc::new(AtomicUsize::new(0));
            let handle = {
                let (counts, keys, stop) = (counts.clone(), keys.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let Ok((stream, _)) = listener.accept() else {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        };
                        stream.set_nonblocking(false).unwrap();
                        serve(stream, &script, &counts, &keys, &polls);
                    }
                })
            };
            Self {
                base,
                counts,
                keys,
                stop,
                handle: Some(handle),
            }
        }

        pub(crate) fn count(&self, what: &str) -> usize {
            self.counts.lock().unwrap().get(what).copied().unwrap_or(0)
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn serve(
        stream: std::net::TcpStream,
        script: &Script,
        counts: &Mutex<HashMap<String, usize>>,
        keys: &Mutex<Vec<String>>,
        polls: &AtomicUsize,
    ) {
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let mut parts = line.split_whitespace();
        let (method, path) = (
            parts.next().unwrap_or("").to_owned(),
            parts.next().unwrap_or("").to_owned(),
        );
        let mut length = 0usize;
        let mut expect = false;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                break;
            }
            let lower = header.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap_or(0);
            }
            if lower.starts_with("expect:") {
                expect = true;
            }
            if lower.starts_with("authorization:") {
                keys.lock().unwrap().push(header.trim().to_owned());
            }
        }
        if expect {
            let _ = writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        let route = if method == "POST" && path == "/api/v1/videos" {
            "submit"
        } else if path == "/api/v1/videos/models" {
            "models"
        } else if path.ends_with("/content?index=0") {
            "content"
        } else {
            "poll"
        };
        *counts.lock().unwrap().entry(route.to_owned()).or_default() += 1;
        let (status, kind, bytes): (u16, String, Vec<u8>) = match route {
            "submit" => match &script.submit {
                Some((status, body)) => (*status, "application/json".into(), body.clone().into_bytes()),
                None => (
                    202,
                    "application/json".into(),
                    br#"{"id":"gen-vid-1-AAAAAAAAAAAAAAAAAAAA","polling_url":"/api/v1/videos/gen-vid-1-AAAAAAAAAAAAAAAAAAAA","status":"pending"}"#.to_vec(),
                ),
            },
            "poll" => {
                let seen = polls.fetch_add(1, Ordering::Relaxed);
                let mut answer = if seen < script.polls {
                    serde_json::json!({"status": "in_progress"})
                } else {
                    script.end.clone()
                };
                answer["id"] = "gen-vid-1-AAAAAAAAAAAAAAAAAAAA".into();
                (200, "application/json".into(), answer.to_string().into_bytes())
            }
            "content" => (200, script.content.0.clone(), script.content.1.clone()),
            _ => (
                200,
                "application/json".into(),
                musializer_core::generation::catalog::tests_live().as_bytes().to_vec(),
            ),
        };
        let head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = writer.write_all(head.as_bytes());
        let _ = writer.write_all(&bytes);
    }

    /// A one-second 64x36 H.264 clip.
    pub(crate) fn tiny_mp4(folder: &Path) -> Vec<u8> {
        let path = folder.join("tiny.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x36:rate=10",
            ])
            .args(["-t", "1", "-pix_fmt", "yuv420p", "-c:v", "libx264"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read(path).unwrap()
    }

    fn request() -> GenerationRequest {
        GenerationRequest::new(
            "google/veo-3.1-lite",
            "fog over a field",
            4,
            &Shape {
                resolution: "720p".into(),
                aspect_ratio: "16:9".into(),
                size: (1280, 720),
            },
        )
    }

    const KEY: &str = "sk-or-v1-canary-H4-0123456789";

    fn spec(fake: &Fake, store: &Path, request: GenerationRequest) -> JobSpec {
        JobSpec {
            name: clip_name(&request),
            request,
            first_png: None,
            last_png: None,
            estimate_usd: Some(0.12),
            secret: Secret::new(KEY.to_owned()),
            endpoint: Endpoint::new(&fake.base).unwrap(),
            store: Store::at(store.to_path_buf()),
            poll: Duration::from_millis(30),
            deadline: Duration::from_secs(20),
            max_bytes: 10 * 1024 * 1024,
        }
    }

    fn completed(cost: f64) -> serde_json::Value {
        serde_json::json!({"status": "completed", "usage": {"cost": cost, "is_byok": false}})
    }

    #[test]
    fn a_job_submits_polls_downloads_verifies_and_is_found_again_for_nothing() {
        let folder = Scratch::new("job-happy");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            submit: None,
            polls: 2,
            end: completed(0.1234),
            content: ("video/mp4".into(), video.clone()),
        });
        let store = folder.path().join("store");
        let outcome = GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {});
        let Outcome::Done(generated) = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(generated.reused, Reuse::New);
        let record = generated.clip.asset.generated.clone().unwrap();
        assert_eq!(record.cost_usd, Some(0.1234));
        assert_eq!(record.estimate_usd, Some(0.12));
        assert_eq!(record.request_sha256, request().digest());
        assert_eq!(generated.clip.asset.sha256, sha256::digest_hex(&video));
        assert_eq!(
            generated.clip.asset.name,
            "fog over a field (Veo 3.1 Lite).mp4"
        );
        assert!(generated.clip.source.starts_with(&store));
        assert_eq!(
            (
                fake.count("submit"),
                fake.count("poll"),
                fake.count("content")
            ),
            (1, 3, 1)
        );
        assert!(fake
            .keys
            .lock()
            .unwrap()
            .iter()
            .all(|key| key.ends_with(KEY)));
        assert!(!store
            .join("jobs")
            .join(format!("{}.json", request().digest()))
            .exists());
        // The same request again: the store's clip, no call at all.
        let again = GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {});
        assert!(matches!(
            &again,
            Outcome::Done(Generated {
                reused: Reuse::Stored,
                ..
            })
        ));
        assert_eq!(
            fake.count("submit") + fake.count("poll") + fake.count("content"),
            5
        );
        // A changed prompt is a new request.
        let mut changed = request();
        changed.prompt.push('!');
        let new = GenerationJob::spawn(spec(&fake, &store, changed)).wait(|_, _| {});
        assert!(
            matches!(
                &new,
                Outcome::Done(Generated {
                    reused: Reuse::New,
                    ..
                })
            ),
            "{new:?}"
        );
        assert_eq!(fake.count("submit"), 2);
        let spending = std::fs::read_to_string(store.join("spending.tsv")).unwrap();
        // A header and the two jobs that were sent; the stored hit spent nothing.
        assert_eq!(spending.lines().count(), 3, "{spending}");
        assert!(!spending.contains(KEY));
    }

    fn failed(outcome: Outcome) -> (String, Billing) {
        match outcome {
            Outcome::Failed { reason, billing } => (reason, billing),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn each_failure_is_named_and_nothing_is_imported() {
        let folder = Scratch::new("job-fail");
        let video = tiny_mp4(folder.path());
        let run = |script: Script| {
            let fake = Fake::start(script);
            let store = folder.path().join(format!(
                "store-{}",
                fake.base.len() + fake.base.as_bytes()[fake.base.len() - 8] as usize
            ));
            let _ = std::fs::remove_dir_all(&store);
            let outcome = GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {});
            let kept = store.join("clips").exists()
                && std::fs::read_dir(store.join("clips")).unwrap().count() > 0;
            (outcome, kept)
        };
        // A failed job with a bill: the bill is said.
        let (outcome, kept) = run(Script {
            submit: None,
            polls: 0,
            end: serde_json::json!({"status": "failed", "error": "content policy", "usage": {"cost": 0.05}}),
            content: ("video/mp4".into(), video.clone()),
        });
        let (reason, billing) = failed(outcome);
        assert_eq!(reason, "the job failed: content policy");
        assert_eq!(billing, Billing::Reported(0.05));
        assert!(!kept);
        // A refused submit: by status and message, and nothing billed.
        let (outcome, _) = run(Script {
            submit: Some((
                402,
                r#"{"error":{"code":402,"message":"Insufficient credits"}}"#.into(),
            )),
            polls: 0,
            end: completed(0.1),
            content: ("video/mp4".into(), video.clone()),
        });
        let (reason, billing) = failed(outcome);
        assert_eq!(
            reason,
            "the OpenRouter account has too little credit (402): Insufficient credits"
        );
        assert_eq!(billing, Billing::Nothing);
        // A download that is not a video, by its type and by its bytes.
        for content in [
            (
                "application/json".to_owned(),
                br#"{"error":"nope"}"#.to_vec(),
            ),
            ("video/mp4".to_owned(), b"not a video at all".to_vec()),
        ] {
            let (outcome, kept) = run(Script {
                submit: None,
                polls: 0,
                end: completed(0.12),
                content,
            });
            let (reason, billing) = failed(outcome);
            assert!(
                reason.contains("not a video") || reason.contains("not a clip"),
                "{reason}"
            );
            assert_eq!(billing, Billing::Reported(0.12));
            assert!(!kept);
        }
        // Larger than allowed.
        let (outcome, kept) = run(Script {
            submit: None,
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), vec![0; 11 * 1024 * 1024]),
        });
        let (reason, _) = failed(outcome);
        assert!(
            reason.contains("larger than the 10485760 bytes"),
            "{reason}"
        );
        assert!(!kept);
    }

    #[test]
    fn a_stop_after_the_submit_says_it_may_be_billed_and_a_second_press_picks_it_up() {
        let folder = Scratch::new("job-stop");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            submit: None,
            polls: 1_000_000,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
        });
        let store = folder.path().join("store");
        let job = GenerationJob::spawn(spec(&fake, &store, request()));
        let started = Instant::now();
        while fake.count("poll") < 2 {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
        job.cancel();
        let stopped_at = Instant::now();
        let outcome = job.wait(|_, _| {});
        assert!(stopped_at.elapsed() < Duration::from_secs(2));
        let Outcome::Stopped { billing } = outcome else {
            panic!("{outcome:?}")
        };
        assert_eq!(billing, Billing::Possibly(Some(0.12)));
        assert!(billing
            .sentence()
            .unwrap()
            .contains("may still be billed (about $0.12)"));
        // The record survives, so the next press polls the same job.
        assert!(Store::at(store.clone()).job(&request().digest()).is_some());
        drop(fake);
        let fake = Fake::start(Script {
            submit: None,
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), tiny_mp4(folder.path())),
        });
        let outcome = GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {});
        assert!(
            matches!(
                &outcome,
                Outcome::Done(Generated {
                    reused: Reuse::Resumed,
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert_eq!(fake.count("submit"), 0, "nothing paid twice");
    }
}
