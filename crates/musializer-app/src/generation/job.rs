//! One generation, on its own thread: submit, poll, download, verify.
//!
//! The window never waits on it: it reads [`GenerationJob::progress`] each
//! frame and takes the [`Outcome`] once. The command line waits on it with
//! [`GenerationJob::wait`]. Either way:
//!
//! - **Nothing is paid for twice.** A clip the store already has for the
//!   request is the outcome at once, with no call; a job submitted earlier
//!   and never downloaded (cancelled, failed to download, the application
//!   closed) is polled again instead of submitted again. The lookup and the
//!   submit are one span under the request's lock in the store
//!   ([`Store::claim`]), so a second caller for the same request — another
//!   window, a treatment run — waits and then finds the first one's job.
//! - **Nothing is half-imported.** The download goes to the job's scratch
//!   folder, is opened by its handle and probed as the clip importer does
//!   (`clips::import`), and only then moves into the store. A download that
//!   is not a video, is cut short, or is larger than allowed is removed and
//!   named.
//! - **A stop is honest about money.** OpenRouter has no call that cancels a
//!   job (2026-11-14), so a stop after the submit stops *waiting*; the job may
//!   still finish and be billed, and the outcome says so with the estimate.
//! - **A lost answer is not "nothing".** An intent is written to the store
//!   before the request leaves and becomes the job's record when the id
//!   arrives. A submit interrupted after the request was written (a Cancel, a
//!   closed window, a dropped connection, a crash) leaves the intent: the job
//!   may exist and be billed, and OpenRouter has no way to find it
//!   (read 2026-11-14). The next ask for that request says so and is
//!   [`Outcome::Unconfirmed`] until the person agrees to send it again
//!   ([`JobSpec::resend`]).

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

/// How many times one ask downloads a finished clip that comes back cut
/// short or refused before it stops and says so (the job is kept).
pub const DOWNLOAD_ATTEMPTS: u32 = 3;

/// How long a job may take before this application stops waiting for it.
pub const DEADLINE: Duration = Duration::from_secs(30 * 60);

/// What a job is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Starting,
    /// Another caller holds the request: it is being sent by them.
    Queued,
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
            Self::Queued => "waiting: another window or run is sending this same request".into(),
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
    /// Sent and never answered: it may have been accepted and billed, and
    /// its job id is unknown.
    Unconfirmed(Option<f64>),
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
            Self::Unconfirmed(usd) => Some(format!(
                "The request was sent and its answer never arrived, so it may have been billed \
                 ({}) for a job this application cannot find: OpenRouter has no way to look one \
                 up. Asking for it again would be a second request, and you are asked first.",
                about(*usd)
            )),
        }
    }
}

/// "about $0.12", or "price unknown".
fn about(usd: Option<f64>) -> String {
    usd.map_or_else(
        || "price unknown".to_owned(),
        |usd| {
            format!(
                "about ${:.2}",
                musializer_core::generation::price::ceil_cents(usd)
            )
        },
    )
}

/// What a person reads when a request they ask for was sent before and no
/// answer arrived: every such send, their sum, and where each is recorded.
/// The caller adds how to send it again.
#[must_use]
pub fn unconfirmed_words(intent: &super::store::Intent, store: &Store) -> String {
    let when = |utc: &str| utc.replace('T', " ").replace('Z', " UTC");
    let ledger = store.root().join("spending.tsv");
    match intent.attempts.as_slice() {
        [only] => format!(
            "This exact request was sent on {} and its answer never arrived, so OpenRouter may \
             have accepted and billed it ({}). OpenRouter has no way to find a job whose id was \
             lost; your activity page at openrouter.ai/activity shows whether it was billed. Every \
             send is in {}.",
            when(&only.sent_utc),
            about(only.estimate_usd),
            ledger.display()
        ),
        attempts => format!(
            "This exact request was sent {} times, first on {} and last on {}, and no answer \
             arrived, so OpenRouter may have accepted and billed each of them ({}). OpenRouter \
             has no way to find a job whose id was lost; your activity page at \
             openrouter.ai/activity shows which were billed. Every send is in {}.",
            attempts.len(),
            when(&attempts[0].sent_utc),
            when(&intent.latest().sent_utc),
            intent.total_usd().map_or_else(
                || "price unknown".to_owned(),
                |usd| format!(
                    "about ${:.2} in all",
                    musializer_core::generation::price::ceil_cents(usd)
                )
            ),
            ledger.display()
        ),
    }
}

/// A treatment run's `--max-spend` (H4d, 2026-11-15): **the most this run may
/// newly send.** The sum of the estimates of the requests this run itself
/// submits never passes it; nothing else counts. A job picked up costs
/// nothing new, and what earlier runs may have spent is reported, not
/// counted.
///
/// It is enforced where money leaves: [`Ceiling::take`] runs under the
/// request's lock immediately before the intent is written, so no order of
/// sections, no section whose request is known only later, and no change
/// another process makes to the store can make the run send more. A send that
/// cannot have been billed (refused by the provider, never left, or stopped
/// before it was written down) gives its estimate back.
#[derive(Debug)]
pub struct Ceiling {
    limit_usd: f64,
    /// As the person typed it.
    shown: String,
    sent: Mutex<(f64, usize)>,
}

impl Ceiling {
    #[must_use]
    pub fn new(limit_usd: f64, shown: &str) -> Self {
        Self {
            limit_usd,
            shown: shown.to_owned(),
            sent: Mutex::new((0.0, 0)),
        }
    }

    /// `$0.1234567`: the ceiling as given, never rounded.
    #[must_use]
    pub fn shown(&self) -> String {
        format!("${}", self.shown)
    }

    #[must_use]
    pub fn limit_usd(&self) -> f64 {
        self.limit_usd
    }

    /// What this run has sent so far: the sum of the estimates, and how many.
    #[must_use]
    pub fn sent(&self) -> (f64, usize) {
        self.sent.lock().map_or((0.0, 0), |sent| *sent)
    }

    /// Takes `estimate` from what is left, or says why it may not be sent.
    ///
    /// # Errors
    ///
    /// The price is unknown, or it would pass the ceiling.
    pub fn take(&self, estimate: Option<f64>) -> Result<(), String> {
        let Some(usd) = estimate else {
            return Err(
                "its price is unknown, and a run with --max-spend sends only what it can count"
                    .into(),
            );
        };
        let mut sent = self
            .sent
            .lock()
            .map_err(|_| "the run's count of what it sent is lost".to_owned())?;
        if sent.0 + usd > self.limit_usd + 1.0e-9 {
            return Err(self.past(sent.0, usd, false));
        }
        sent.0 += usd;
        sent.1 += 1;
        Ok(())
    }

    /// "it would pass the ceiling: …", in one place; `forecast` when it is
    /// said before anything was sent.
    #[must_use]
    pub fn past(&self, sent: f64, more: f64, forecast: bool) -> String {
        format!(
            "it would pass the ceiling: {} ${:.2}, this request is ${:.2} more, {} allowed \
             (--max-spend counts what this run sends)",
            if forecast {
                "by then this run would have sent"
            } else {
                "this run has sent"
            },
            musializer_core::generation::price::ceil_cents(sent),
            musializer_core::generation::price::ceil_cents(more),
            self.shown()
        )
    }

    /// A taken estimate whose request cannot have been billed.
    fn give_back(&self, estimate: Option<f64>) {
        if let (Some(usd), Ok(mut sent)) = (estimate, self.sent.lock()) {
            sent.0 = (sent.0 - usd).max(0.0);
            sent.1 = sent.1.saturating_sub(1);
        }
    }
}

/// What a person reads when a request has reached
/// [`super::store::MAX_UNANSWERED_SENDS`] unanswered sends: it is not sent
/// again at all, by the window or a run, until the cause is found and the
/// record removed. `None` below the bound.
#[must_use]
pub fn too_many_words(intent: &super::store::Intent, store: &Store) -> Option<String> {
    (intent.attempts.len() >= super::store::MAX_UNANSWERED_SENDS).then(|| {
        format!(
            "This exact request was sent {} times and no answer ever arrived ({}, each possibly \
             billed). Something is wrong between this machine and OpenRouter (a proxy, a firewall \
             or the network cutting the connection after the request leaves, or the provider \
             failing), so it is not sent again, by the window or by a run, with or without \
             --resend-unconfirmed. Your activity page at openrouter.ai/activity shows what was \
             billed; once the cause is found, remove {} to send it anew. Every send is in {}",
            intent.attempts.len(),
            intent.total_usd().map_or_else(
                || "price unknown".to_owned(),
                |usd| format!(
                    "about ${:.2} in all",
                    musializer_core::generation::price::ceil_cents(usd)
                )
            ),
            store.intent_path(&intent.request_sha256).display(),
            store.root().join("spending.tsv").display()
        )
    })
}

/// A fresh attempt's id: its time to the nanosecond and this process, so two
/// sends of one request never share one.
fn attempt_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{nanos}p{}", std::process::id())
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
#[allow(
    clippy::large_enum_variant,
    reason = "one per job, made once and moved once into the track"
)]
pub enum Outcome {
    Done(Generated),
    Failed {
        reason: String,
        billing: Billing,
    },
    Stopped {
        billing: Billing,
    },
    /// Nothing was sent now: an earlier send of this request may have been
    /// billed (its intent is left over), and this ask did not agree to send
    /// it again.
    Unconfirmed(super::store::Intent),
    /// Nothing was sent, by rule, and the reason: the run's `--max-spend`
    /// would be passed.
    Refused(String),
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
    /// The id of the newest unconfirmed send the person agreed to send
    /// again after, told every earlier one may have been billed. Any other
    /// state of the request's unconfirmed sends stops the job before anything
    /// is sent.
    pub resend: Option<String>,
    /// A treatment run's `--max-spend`; the window has none (each Send is
    /// its own consent at a stated price).
    pub ceiling: Option<Arc<Ceiling>>,
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

    // The request is this caller's from the lookup until its job id is on
    // disk; a second caller waits here, then finds that job.
    let claim = match store.claim(&digest, stop, || set_stage(shared, Stage::Queued)) {
        Ok(Some(claim)) => claim,
        Ok(None) => {
            return Outcome::Stopped {
                billing: Billing::Nothing,
            }
        }
        Err(reason) => {
            return Outcome::Failed {
                reason,
                billing: Billing::Nothing,
            }
        }
    };

    // 1. The store has it: nothing is sent.
    if let Some((path, record)) = store.find(&digest) {
        claim.release();
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

    let (work_name, work) = match store.private_work(&digest) {
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
    let outcome = run_with(spec, &client, &digest, claim, stop, shared);
    store.clear_work(&work_name);
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
        Outcome::Unconfirmed(_) => ("held".to_owned(), None),
        Outcome::Refused(_) => ("refused".to_owned(), None),
    };
    let verdict = match &outcome {
        Outcome::Failed {
            billing: Billing::Unconfirmed(_),
            ..
        }
        | Outcome::Stopped {
            billing: Billing::Unconfirmed(_),
        } => format!("{verdict}, answer lost, possibly billed"),
        Outcome::Failed {
            billing: Billing::Possibly(_),
            ..
        }
        | Outcome::Stopped {
            billing: Billing::Possibly(_),
        } => format!("{verdict}, possibly billed"),
        _ => verdict,
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
        } | Outcome::Unconfirmed(_)
            | Outcome::Refused(_)
    ) {
        let job_id = shared
            .lock()
            .ok()
            .and_then(|shared| shared.job_id.clone())
            .unwrap_or_else(|| "-".to_owned());
        // The money has left by now; a ledger that cannot take the line is
        // said, and the job record or intent still holds the request.
        if let Err(reason) = store.log_spending(&[
            &now_utc(),
            &digest,
            &job_id,
            &spec.request.model,
            &spec.request.seconds.to_string(),
            &money(spec.estimate_usd),
            &money(cost),
            &verdict,
        ]) {
            eprintln!("warning: the outcome of a paid request is not in the ledger: {reason}");
        }
    }
    outcome
}

fn run_with(
    spec: &JobSpec,
    client: &Client<'_>,
    digest: &str,
    claim: super::store::Claim,
    stop: &AtomicBool,
    shared: &Mutex<Shared>,
) -> Outcome {
    let store = &spec.store;
    let possibly = Billing::Possibly(spec.estimate_usd);

    // 2. An earlier submit for the same request is picked up, not repeated.
    let (job_id, resumed) = if let Some(record) = store.job(digest) {
        claim.release();
        (record.job_id, true)
    } else {
        // Earlier sends whose answers never arrived: nothing goes out unless
        // the person agreed to send it again, told of every one of them.
        match store.intent(digest) {
            Ok(None) => {}
            Ok(Some(intent)) => {
                if let Some(words) = too_many_words(&intent, store) {
                    claim.release();
                    return Outcome::Refused(words);
                }
                if spec.resend.as_deref() != Some(intent.latest().id.as_str()) {
                    claim.release();
                    return Outcome::Unconfirmed(intent);
                }
            }
            // A record that cannot be read is not "no record": it holds
            // the request until the person repairs or removes it.
            Err(unreadable) => {
                claim.release();
                return Outcome::Refused(unreadable.words());
            }
        }
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
        // Written down before it leaves, beside any earlier send still
        // unanswered: if this answer is lost too, the next ask finds both
        // and says each may have been billed.
        let attempt = super::store::Attempt {
            id: attempt_id(),
            sent_utc: now_utc(),
            estimate_usd: spec.estimate_usd,
        };
        let sending = attempt.id.clone();
        let sent_utc = attempt.sent_utc.clone();
        // A run's ceiling, under the request's lock and immediately before
        // the send is written down: what this run sends never passes it.
        if let Some(ceiling) = &spec.ceiling {
            if let Err(reason) = ceiling.take(spec.estimate_usd) {
                claim.release();
                return Outcome::Refused(reason);
            }
        }
        let give_back = || {
            if let Some(ceiling) = &spec.ceiling {
                ceiling.give_back(spec.estimate_usd);
            }
        };
        if let Err(reason) = store.add_attempt(
            &claim,
            digest,
            &spec.request.model,
            spec.request.seconds,
            attempt,
        ) {
            give_back();
            return Outcome::Failed {
                reason: format!(
                    "the request could not be written down before sending ({reason}), so it was \
                     not sent"
                ),
                billing: Billing::Nothing,
            };
        }
        // The person's ledger has the send before it leaves: a crash in the
        // submit leaves this line and no outcome, which reads as what it is.
        // A line that cannot be written durably stops the send (H4d): the
        // person's record of what may have cost money would miss it.
        if let Err(reason) = store.log_spending(&[
            &sent_utc,
            digest,
            "-",
            &spec.request.model,
            &spec.request.seconds.to_string(),
            &money(spec.estimate_usd),
            "unknown",
            "sending, billed if accepted",
        ]) {
            give_back();
            if let Err(unrecorded) = store.resolve_attempt(&claim, digest, &sending) {
                eprintln!("warning: an unsent request stays on the record: {unrecorded}");
            }
            return Outcome::Failed {
                reason: format!(
                    "the line that records this send in the ledger could not be written ({reason}), \
                     so it was not sent"
                ),
                billing: Billing::Nothing,
            };
        }
        let unconfirmed = Billing::Unconfirmed(spec.estimate_usd);
        let state = match client.submit(&body_path) {
            Ok(state) => state,
            Err(failed) if !failed.may_be_billed => {
                // Refused by the provider, or never sent: no job exists.
                give_back();
                if let Err(reason) = store.resolve_attempt(&claim, digest, &sending) {
                    eprintln!(
                        "warning: a refused send could not be taken off the record: {reason}"
                    );
                }
                return Outcome::Failed {
                    reason: failed.error.to_string(),
                    billing: Billing::Nothing,
                };
            }
            // The intent stays: the job may exist, and its id is unknown.
            Err(super::client::Submitted {
                error: ClientError::Stopped,
                ..
            }) => {
                return Outcome::Stopped {
                    billing: unconfirmed,
                }
            }
            Err(failed) => {
                return Outcome::Failed {
                    reason: failed.error.to_string(),
                    billing: unconfirmed,
                }
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
        match store.record_job(&record) {
            Ok(()) => {
                // This send is the job now; earlier unanswered ones stay.
                if let Err(reason) = store.resolve_attempt(&claim, digest, &sending) {
                    eprintln!("warning: an answered send stays on the record: {reason}");
                }
            }
            // The attempt stays, so a lost record still stops the next ask
            // from paying again silently.
            Err(reason) => eprintln!("warning: the job could not be recorded: {reason}"),
        }
        // Its id is on disk: the next caller for this request polls it.
        claim.release();
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
                // Only this job's record: a newer one for the request (a
                // caller that came after another forgot this job) is paid.
                store.forget_job_if(digest, &job_id, stop);
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
        // The provider says the job is over without a clip: its record goes,
        // and the next ask is a new request (and a new bill). Only if the
        // record is still this job's: another caller polling the same job
        // may have forgotten it already, and a third submitted a new one.
        store.forget_job_if(digest, &job_id, stop);
        let reason = if state.status == "expired" {
            "the job expired at OpenRouter before its clip was downloaded; Generate again asks \
             (and pays) anew"
                .to_owned()
        } else {
            format!(
                "the job {}{}",
                state.status,
                state
                    .error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            )
        };
        return Outcome::Failed { reason, billing };
    }

    // 4. Download and verify, a few times: the job is finished and paid
    // for, so a copy cut short or refused is fetched again, and the job's
    // record stays until a copy is kept. Forgetting it here would make the
    // next ask pay for a new generation of a clip that exists.
    let part = client.work.join("clip.part");
    let record = spec
        .request
        .provenance(&now_utc(), spec.estimate_usd, state.cost_usd);
    let mut refused = Vec::new();
    let clip = loop {
        if refused.len() as u32 >= DOWNLOAD_ATTEMPTS {
            return Outcome::Failed {
                reason: format!(
                    "the finished clip was downloaded {} times and refused each time (last: {}); \
                     its job is kept, so Generate again downloads it again without paying again",
                    refused.len(),
                    refused.last().map_or("", String::as_str)
                ),
                billing,
            };
        }
        if !refused.is_empty() && !pause(spec.poll, stop) {
            return Outcome::Stopped { billing };
        }
        set_stage(shared, Stage::Downloading);
        match client.download(&job_id, &part, spec.max_bytes) {
            Ok(_) => {}
            Err(ClientError::Stopped) => {
                let _ = std::fs::remove_file(&part);
                return Outcome::Stopped { billing };
            }
            // Larger than allowed stays larger: fetching it again only costs
            // the bandwidth. The job is kept all the same.
            Err(error @ ClientError::TooLarge(_)) => {
                let _ = std::fs::remove_file(&part);
                return Outcome::Failed {
                    reason: format!("the clip could not be downloaded: {error}"),
                    billing,
                };
            }
            Err(error) => {
                let _ = std::fs::remove_file(&part);
                refused.push(format!("the clip could not be downloaded: {error}"));
                continue;
            }
        }
        set_stage(shared, Stage::Verifying);
        match import(&part, record.clone(), &spec.name) {
            Ok(clip) => break clip,
            Err(reason) => {
                let _ = std::fs::remove_file(&part);
                refused.push(format!(
                    "the download is not a clip this application can play: {reason}"
                ));
            }
        }
    };
    // Kept under the request's lock: two callers that polled the same job
    // both download it, and the second finds the first one's copy.
    let claim = match store.claim(digest, stop, || {}) {
        Ok(Some(claim)) => claim,
        Ok(None) => {
            let _ = std::fs::remove_file(&part);
            return Outcome::Stopped { billing };
        }
        Err(reason) => {
            return Outcome::Failed {
                reason: format!("the clip could not be kept: {reason}"),
                billing,
            }
        }
    };
    let kept = match store.find(digest) {
        Some((path, _)) => {
            let _ = std::fs::remove_file(&part);
            Ok(path)
        }
        None => store.keep(&part, &record, &clip.asset.sha256),
    };
    let kept = match kept {
        Ok(kept) => kept,
        Err(reason) => {
            return Outcome::Failed {
                reason: format!("the clip could not be kept: {reason}"),
                billing,
            }
        }
    };
    store.forget_job_held(&claim, digest, &job_id);
    // The clip is in hand: no earlier unanswered send can make the person pay
    // for it again (each stays in the ledger).
    store.clear_intent_held(&claim, digest);
    claim.release();
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
    #[derive(Clone, Default)]
    pub(crate) struct Script {
        /// How long the submit takes to answer (after it was read).
        pub submit_delay: Duration,
        /// The first this many submits are read and accepted, and the
        /// connection then closes with no answer.
        pub drop_submits: usize,
        /// The first this many downloads are the clip's first half.
        pub truncated_downloads: usize,
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
        if method == "POST" && path == "/api/v1/videos" {
            std::thread::sleep(script.submit_delay);
            let seen = counts.lock().unwrap().get("submit").copied().unwrap_or(0);
            if seen < script.drop_submits {
                *counts.lock().unwrap().entry("submit".into()).or_default() += 1;
                return;
            }
        }
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
            "content" => {
                let seen = counts.lock().unwrap().get("content").copied().unwrap_or(0);
                let bytes = &script.content.1;
                let bytes = if seen <= script.truncated_downloads {
                    bytes[..bytes.len() / 2].to_vec()
                } else {
                    bytes.clone()
                };
                (200, script.content.0.clone(), bytes)
            }
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
        spec_at(&fake.base, store, request)
    }

    fn spec_at(base: &str, store: &Path, request: GenerationRequest) -> JobSpec {
        JobSpec {
            name: clip_name(&request),
            request,
            first_png: None,
            last_png: None,
            estimate_usd: Some(0.12),
            secret: Secret::new(KEY.to_owned()),
            endpoint: Endpoint::new(base).unwrap(),
            store: Store::at(store.to_path_buf()),
            poll: Duration::from_millis(30),
            deadline: Duration::from_secs(20),
            max_bytes: 10 * 1024 * 1024,
            resend: None,
            ceiling: None,
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
            ..Script::default()
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
        // A header, and for each of the two jobs sent a line as it left and
        // one when it ended; the stored hit spent nothing.
        assert_eq!(spending.lines().count(), 5, "{spending}");
        assert_eq!(spending.matches("sending, billed if accepted").count(), 2);
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
            ..Script::default()
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
            ..Script::default()
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
                ..Script::default()
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
            ..Script::default()
        });
        let (reason, _) = failed(outcome);
        assert!(
            reason.contains("larger than the 10485760 bytes"),
            "{reason}"
        );
        assert!(!kept);
    }

    #[test]
    fn two_callers_for_one_request_pay_once() {
        // A slow submit holds the request's lock; the second caller, started
        // while it is in flight, waits and then polls the job the first one
        // recorded. Without the lock both submit.
        let folder = Scratch::new("job-two");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            submit_delay: Duration::from_millis(600),
            polls: 2,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
            ..Script::default()
        });
        let store = folder.path().join("store");
        let first = GenerationJob::spawn(spec(&fake, &store, request()));
        let started = Instant::now();
        while fake.count("submit") == 0 && first.progress().0 != Stage::Submitting {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let second = GenerationJob::spawn(spec(&fake, &store, request()));
        // The second shows it is waiting on the first.
        let started = Instant::now();
        let mut queued = false;
        while !queued && started.elapsed() < Duration::from_secs(2) {
            queued = second.progress().0 == Stage::Queued;
            std::thread::sleep(Duration::from_millis(5));
        }
        let (a, b) = (first.wait(|_, _| {}), second.wait(|_, _| {}));
        assert_eq!(fake.count("submit"), 1, "one request, paid once");
        assert!(queued, "the second caller waited on the first");
        assert!(
            matches!(
                &a,
                Outcome::Done(Generated {
                    reused: Reuse::New,
                    ..
                })
            ),
            "{a:?}"
        );
        assert!(
            matches!(
                &b,
                Outcome::Done(Generated {
                    reused: Reuse::Resumed | Reuse::Stored,
                    ..
                })
            ),
            "{b:?}"
        );
        assert!(Store::at(store.clone()).find(&request().digest()).is_some());
        assert!(!Store::at(store).in_flight(&request().digest()));
    }

    #[test]
    fn a_lost_answer_may_be_billed_and_is_sent_again_only_when_agreed() {
        let folder = Scratch::new("job-lost");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            drop_submits: 1,
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
            ..Script::default()
        });
        let store = folder.path().join("store");
        let digest = request().digest();
        // Accepted, and the answer never arrives: it may have been billed.
        let (reason, billing) =
            failed(GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {}));
        assert_eq!(billing, Billing::Unconfirmed(Some(0.12)), "{reason}");
        assert!(billing
            .sentence()
            .unwrap()
            .contains("may have been billed (about $0.12)"));
        let intent = Store::at(store.clone())
            .intent(&digest)
            .unwrap()
            .expect("the intent stays");
        assert_eq!(fake.count("submit"), 1);
        // Asked again without agreeing: nothing is sent, and why is said.
        let held = GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {});
        let Outcome::Unconfirmed(seen) = held else {
            panic!("{held:?}")
        };
        assert_eq!(seen, intent);
        assert!(unconfirmed_words(&seen, &Store::at(store.clone()))
            .contains("may have accepted and billed it (about $0.12)"));
        assert_eq!(fake.count("submit"), 1, "nothing resent silently");
        // Agreed: sent anew, and the intent becomes the job.
        let mut agreed = spec(&fake, &store, request());
        agreed.resend = Some(intent.latest().id.clone());
        let outcome = GenerationJob::spawn(agreed).wait(|_, _| {});
        assert!(
            matches!(
                &outcome,
                Outcome::Done(Generated {
                    reused: Reuse::New,
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert_eq!(fake.count("submit"), 2);
        assert!(Store::at(store).intent(&digest).unwrap().is_none());
    }

    /// H4c: a send agreed after a lost one is lost too. Both stay on record,
    /// the words name both and their sum, and only a clip in hand clears them.
    #[test]
    fn two_lost_answers_both_stay_until_the_clip_is_in_hand() {
        let folder = Scratch::new("job-lost-twice");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            drop_submits: 2,
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
            ..Script::default()
        });
        let store = folder.path().join("store");
        let digest = request().digest();
        let (_, billing) =
            failed(GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {}));
        assert_eq!(billing, Billing::Unconfirmed(Some(0.12)));
        let first = Store::at(store.clone()).intent(&digest).unwrap().unwrap();
        let mut again = spec(&fake, &store, request());
        again.resend = Some(first.latest().id.clone());
        let (_, billing) = failed(GenerationJob::spawn(again).wait(|_, _| {}));
        assert_eq!(billing, Billing::Unconfirmed(Some(0.12)));
        let both = Store::at(store.clone()).intent(&digest).unwrap().unwrap();
        assert_eq!(
            both.attempts.len(),
            2,
            "the second send did not replace the first"
        );
        assert_eq!(both.attempts[0], first.attempts[0]);
        // Agreeing to the first one only is not agreeing to both.
        let mut stale = spec(&fake, &store, request());
        stale.resend = Some(first.latest().id.clone());
        let held = GenerationJob::spawn(stale).wait(|_, _| {});
        let Outcome::Unconfirmed(seen) = held else {
            panic!("{held:?}")
        };
        let words = unconfirmed_words(&seen, &Store::at(store.clone()));
        assert!(words.contains("was sent 2 times"), "{words}");
        assert!(words.contains("(about $0.24 in all)"), "{words}");
        assert!(words.contains("spending.tsv"), "{words}");
        assert_eq!(fake.count("submit"), 2);
        let mut agreed = spec(&fake, &store, request());
        agreed.resend = Some(both.latest().id.clone());
        let outcome = GenerationJob::spawn(agreed).wait(|_, _| {});
        assert!(matches!(&outcome, Outcome::Done(_)), "{outcome:?}");
        assert_eq!(fake.count("submit"), 3);
        assert!(Store::at(store.clone()).intent(&digest).unwrap().is_none());
        let ledger = std::fs::read_to_string(store.join("spending.tsv")).unwrap();
        assert_eq!(
            ledger
                .matches("failed, answer lost, possibly billed")
                .count(),
            2,
            "{ledger}"
        );
        assert_eq!(ledger.matches("sending, billed if accepted").count(), 3);
    }

    /// H4d: a run's ceiling counts what this run sends, where it leaves. A
    /// second request past it is refused with nothing written and nothing
    /// sent; a job recorded before is picked up under a ceiling of 0; a send
    /// the provider refused gives its estimate back.
    #[test]
    fn a_runs_ceiling_counts_only_what_it_sends() {
        let folder = Scratch::new("job-ceiling");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
            ..Script::default()
        });
        let store = folder.path().join("store");
        let ceiling = Arc::new(Ceiling::new(0.12, "0.12"));
        let mut first = spec(&fake, &store, request());
        first.ceiling = Some(ceiling.clone());
        let outcome = GenerationJob::spawn(first).wait(|_, _| {});
        assert!(matches!(&outcome, Outcome::Done(_)), "{outcome:?}");
        assert_eq!(ceiling.sent(), (0.12, 1));
        let mut other = request();
        other.prompt.push_str(" again");
        let mut second = spec(&fake, &store, other.clone());
        second.ceiling = Some(ceiling.clone());
        let Outcome::Refused(why) = GenerationJob::spawn(second).wait(|_, _| {}) else {
            panic!("the second request was not refused")
        };
        assert_eq!(
            why,
            "it would pass the ceiling: this run has sent $0.12, this request is $0.12 more, \
             $0.12 allowed (--max-spend counts what this run sends)"
        );
        assert_eq!(fake.count("submit"), 1);
        assert!(Store::at(store.clone())
            .intent(&other.digest())
            .unwrap()
            .is_none());
        // A job recorded before: picked up under a ceiling of nothing.
        let mut third = request();
        third.prompt.push_str(" picked up");
        Store::at(store.clone())
            .record_job(&JobRecord {
                schema: String::new(),
                request_sha256: third.digest(),
                job_id: "gen-vid-1-AAAAAAAAAAAAAAAAAAAA".into(),
                model: third.model.clone(),
                seconds: 4,
                submitted_utc: "2026-11-15T10:00:00Z".into(),
                estimate_usd: Some(0.12),
            })
            .unwrap();
        let none = Arc::new(Ceiling::new(0.0, "0"));
        let mut pickup = spec(&fake, &store, third);
        pickup.ceiling = Some(none.clone());
        let outcome = GenerationJob::spawn(pickup).wait(|_, _| {});
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
        assert_eq!((none.sent(), fake.count("submit")), ((0.0, 0), 1));
        // Refused by the provider: nothing can be billed, so it is given back.
        let refusing = Fake::start(Script {
            submit: Some((
                402,
                r#"{"error":{"code":402,"message":"Insufficient credits"}}"#.into(),
            )),
            ..Script::default()
        });
        let back = Arc::new(Ceiling::new(0.12, "0.12"));
        let mut refused = spec(&refusing, &folder.path().join("store-402"), request());
        refused.ceiling = Some(back.clone());
        let (_, billing) = failed(GenerationJob::spawn(refused).wait(|_, _| {}));
        assert_eq!(billing, Billing::Nothing);
        assert_eq!(back.sent(), (0.0, 0));
    }

    /// H4d: an intent record that cannot be read (grown past 64 KiB, or not
    /// a record) holds the request instead of reading as "no intent" and being
    /// replaced; three unanswered sends stop the request even when the person
    /// agreed to send it again.
    #[test]
    fn an_unreadable_intent_and_too_many_unanswered_sends_hold_the_request() {
        let folder = Scratch::new("job-held-hard");
        let fake = Fake::start(Script::default());
        let store = folder.path().join("store");
        let digest = request().digest();
        let intents = store.join("intents");
        std::fs::create_dir_all(&intents).unwrap();
        let path = intents.join(format!("{digest}.json"));
        let attempt = |id: &str| super::super::store::Attempt {
            id: id.into(),
            sent_utc: "2026-11-15T10:00:00Z".into(),
            estimate_usd: Some(0.12),
        };
        let record = |attempts: Vec<super::super::store::Attempt>| {
            serde_json::json!({
                "schema": "musializer.video-intent/v2", "request_sha256": digest,
                "model": "google/veo-3.1-lite", "seconds": 4, "attempts": attempts,
            })
            .to_string()
        };
        let mut oversized = record(vec![attempt("a1")]);
        oversized.push_str(&" ".repeat(70 * 1024));
        for (bytes, why) in [
            (oversized, "larger than 65536 bytes"),
            ("{not json".to_owned(), "not a record of sends"),
        ] {
            std::fs::write(&path, &bytes).unwrap();
            let mut agreed = spec(&fake, &store, request());
            agreed.resend = Some("a1".into());
            let Outcome::Refused(words) = GenerationJob::spawn(agreed).wait(|_, _| {}) else {
                panic!("an unreadable intent did not hold the request")
            };
            assert!(words.contains("cannot be read"), "{words}");
            assert!(words.contains(why), "{words}");
            assert!(words.contains("remove the file"), "{words}");
            assert_eq!(fake.count("submit"), 0);
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                bytes,
                "never replaced"
            );
        }
        // Three unanswered sends: held, whatever the person agreed to.
        std::fs::write(
            &path,
            record(vec![attempt("a1"), attempt("a2"), attempt("a3")]),
        )
        .unwrap();
        let mut agreed = spec(&fake, &store, request());
        agreed.resend = Some("a3".into());
        let Outcome::Refused(words) = GenerationJob::spawn(agreed).wait(|_, _| {}) else {
            panic!("three unanswered sends did not hold the request")
        };
        assert!(
            words.starts_with(
                "This exact request was sent 3 times and no answer ever arrived (about $0.36 in \
                 all, each possibly billed). Something is wrong between this machine and \
                 OpenRouter"
            ),
            "{words}"
        );
        assert_eq!(fake.count("submit"), 0);
        // Two are below the bound: the agreed resend goes.
        std::fs::write(&path, record(vec![attempt("a1"), attempt("a2")])).unwrap();
        let mut agreed = spec(&fake, &store, request());
        agreed.resend = Some("a2".into());
        agreed.deadline = Duration::from_millis(200);
        let _ = GenerationJob::spawn(agreed).wait(|_, _| {});
        assert_eq!(fake.count("submit"), 1);
    }

    /// H4d: a ledger that cannot take the line recording a send stops it:
    /// nothing reaches the provider, nothing is left on the record, and the
    /// run's ceiling gets the estimate back.
    #[test]
    fn an_unwritable_ledger_stops_the_send() {
        let folder = Scratch::new("job-ledger");
        let fake = Fake::start(Script::default());
        let store = folder.path().join("store");
        std::fs::create_dir_all(store.join("spending.tsv")).unwrap();
        let ceiling = Arc::new(Ceiling::new(0.12, "0.12"));
        let mut job = spec(&fake, &store, request());
        job.ceiling = Some(ceiling.clone());
        let (reason, billing) = failed(GenerationJob::spawn(job).wait(|_, _| {}));
        assert!(
            reason
                .starts_with("the line that records this send in the ledger could not be written"),
            "{reason}"
        );
        assert!(reason.ends_with("so it was not sent"), "{reason}");
        assert_eq!(billing, Billing::Nothing);
        assert_eq!(fake.count("submit"), 0);
        assert!(Store::at(store)
            .intent(&request().digest())
            .unwrap()
            .is_none());
        assert_eq!(ceiling.sent(), (0.0, 0));
    }

    #[test]
    fn a_submit_that_never_left_bills_nothing_and_leaves_no_intent() {
        let folder = Scratch::new("job-unsent");
        let store = folder.path().join("store");
        // A port nothing listens on: curl is refused before a byte is sent.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!(
            "http://127.0.0.1:{}/api/v1",
            closed.local_addr().unwrap().port()
        );
        drop(closed);
        let mut job = spec_at(&base, &store, request());
        job.deadline = Duration::from_secs(2);
        let (reason, billing) = failed(GenerationJob::spawn(job).wait(|_, _| {}));
        assert_eq!(billing, Billing::Nothing, "{reason}");
        assert!(Store::at(store)
            .intent(&request().digest())
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_refused_download_keeps_its_job_and_the_next_ask_downloads_it_again() {
        let folder = Scratch::new("job-redownload");
        let video = tiny_mp4(folder.path());
        let fake = Fake::start(Script {
            truncated_downloads: DOWNLOAD_ATTEMPTS as usize,
            polls: 0,
            end: completed(0.12),
            content: ("video/mp4".into(), video),
            ..Script::default()
        });
        let store = folder.path().join("store");
        let (reason, billing) =
            failed(GenerationJob::spawn(spec(&fake, &store, request())).wait(|_, _| {}));
        assert!(
            reason.contains("downloaded 3 times and refused each time"),
            "{reason}"
        );
        assert!(reason.contains("Generate again downloads it again without paying again"));
        assert_eq!(billing, Billing::Reported(0.12));
        assert_eq!(fake.count("content"), 3);
        assert!(
            Store::at(store.clone()).job(&request().digest()).is_some(),
            "the paid job is kept"
        );
        // The next ask polls the same job and downloads it whole.
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
        assert_eq!(fake.count("submit"), 1, "paid once");
        assert_eq!(fake.count("content"), 4);
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
            ..Script::default()
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
            ..Script::default()
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
