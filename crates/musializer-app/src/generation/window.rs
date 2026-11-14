//! Generating from the window (VG H4): the section card's Generate.
//!
//! The card (`ui/panels/tune/cards/generate.rs`) draws from a [`CardView`]
//! the composition root hands the shell each frame, and asks for work with
//! `ShellCommand`s that land here: [`Generations::start`] renders the
//! section's frames through the export's still path (the same progress screen
//! a saved still shows, seconds), builds the request, checks the price it
//! carries against the one the button showed, and starts the job on its own
//! thread; [`Generations::tend`] takes a finished job's clip into the track
//! that asked for it and places it in its section. The window never waits on
//! the network.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use musializer_core::assist::secret::Secret;
use musializer_core::generation::catalog::Catalog;
use musializer_core::generation::price;
use musializer_core::generation::request::{FrameImage, GenerationRequest};
use musializer_core::project::clip::ClipPlacement;
use musializer_core::project::sha256;
use musializer_core::project::transition::Transition;
use musializer_core::scene::SceneId;
use musializer_core::ui::notice::Severity;
use musializer_runtime::font::Faces;
use raylib::prelude::{RaylibAudio, RaylibHandle, RaylibThread};

use super::job::{Billing, GenerationJob, JobSpec, Outcome, Reuse, Stage};
use super::store::Store;
use crate::scene_host::SceneRenderer;

/// The notice's title when a request is held because an earlier send of it
/// may have been billed.
const HELD_TITLE: &str = "An earlier send may have been billed";

/// What the card's next Send does after that warning.
const AGAIN: &str = "Press Generate again to send it anew: that is a second request, and it \
                     is billed too.";

/// What the card asks for when its Send is pressed.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerateOrder {
    pub track_slot: usize,
    pub cue_id: u64,
    pub prompt: String,
    pub model: String,
    pub seconds: u32,
    /// Start on this section's first frame, as drawn.
    pub first: bool,
    /// End on the next section's first frame, as drawn.
    pub last: bool,
    /// The price the button showed, which the request must still have.
    pub shown: Option<f64>,
    /// The newest unconfirmed send the card warned about, by its id: the
    /// request was sent before and no answer arrived, so each send may have
    /// been billed. Sending it again needs this to be the request's newest
    /// unconfirmed send still (the card showed the warning for exactly those).
    pub resend: Option<String>,
}

/// A request whose earlier send may have been billed, for the card: its
/// warning, and the digest a second Send agrees to send again.
#[derive(Clone, Debug, PartialEq)]
pub struct UnconfirmedView {
    pub digest: String,
    /// The newest unconfirmed send's id, which a second Send agrees to.
    pub attempt: String,
    pub words: String,
}

/// A job's state, for the card.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningView {
    pub cue_id: u64,
    pub stage: String,
    pub elapsed: Duration,
    pub model: String,
    pub seconds: u32,
    pub estimate: Option<f64>,
    pub cancelling: bool,
}

/// How the last job for a section ended, for the card.
#[derive(Clone, Debug, PartialEq)]
pub struct LastOutcome {
    pub severity: Severity,
    pub words: String,
}

/// Everything the card reads, owned, refreshed each frame.
#[derive(Clone, Debug)]
pub struct CardView {
    pub catalog: Option<Arc<Catalog>>,
    /// "price list of 2026-11-14" or why there is none.
    pub prices: String,
    pub refreshing: bool,
    /// `Ok` when a key is stored; the sentence where it is set otherwise.
    pub key: Result<(), String>,
    pub running: Vec<RunningView>,
    pub last: HashMap<u64, LastOutcome>,
    /// Sections whose request was sent before and never answered.
    pub unconfirmed: HashMap<u64, UnconfirmedView>,
    /// The store's folder, for the card's hint.
    pub kept_in: Option<PathBuf>,
    /// The `generate:` report line.
    pub report: String,
}

impl Default for CardView {
    fn default() -> Self {
        Self {
            catalog: None,
            prices: "no price list".to_owned(),
            refreshing: false,
            key: Err(super::NO_KEY.to_owned()),
            running: Vec::new(),
            last: HashMap::new(),
            unconfirmed: HashMap::new(),
            kept_in: None,
            report: "none".to_owned(),
        }
    }
}

impl CardView {
    #[must_use]
    pub fn running_for(&self, cue_id: u64) -> Option<&RunningView> {
        self.running.iter().find(|running| running.cue_id == cue_id)
    }
}

struct Running {
    order: GenerateOrder,
    job: GenerationJob,
    estimate: Option<f64>,
    /// The section it is for, as it was when Send was pressed.
    target: Target,
    /// The request's digest, for a warning when its answer is lost.
    digest: String,
}

/// The project a section belongs to: its slot, its audio file and the
/// project file, as the look cards' owner is (a slot alone is reused by the
/// next project opened there).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Owner {
    slot: usize,
    audio: PathBuf,
    project: Option<PathBuf>,
}

impl Owner {
    fn of(workspace: &crate::workspace::Workspace, slot: usize) -> Option<Self> {
        let track = workspace.get(slot)?;
        Some(Self {
            slot,
            audio: track.file_path.clone(),
            project: track.project_path.clone(),
        })
    }
}

/// The section a generated clip is for, by identity rather than position:
/// a followed treatment recompiles the plan and hands out the same positional
/// cue ids to other sections.
#[derive(Clone, Debug)]
struct Target {
    owner: Owner,
    audio_sha256: String,
    cue_id: u64,
    /// The section's start and end when Send was pressed.
    span: (f64, f64),
    /// A treatment was followed at some frame while the job ran: the plan
    /// is the file's, whatever the ids say.
    followed: bool,
}

/// Why a finished clip goes into the library unplaced.
#[derive(Debug, PartialEq)]
enum Unplaced {
    /// The project is not open any more: the clip is only in the store.
    Gone,
    /// In the project, not in a section: why.
    Kept(String),
}

/// Whether the clip may still play in the section it was asked for.
fn placement(
    target: &Target,
    owner: Option<&Owner>,
    audio_sha256: &str,
    cues: &[musializer_core::project::scene_switch::SceneSwitchCue],
    followed_now: bool,
) -> Result<(), Unplaced> {
    if owner != Some(&target.owner) || audio_sha256 != target.audio_sha256 {
        return Err(Unplaced::Gone);
    }
    if followed_now || target.followed {
        return Err(Unplaced::Kept(
            "a treatment was followed while it was made, and the followed file decides what each \
             section plays"
                .into(),
        ));
    }
    let Some(cue) = cues.iter().find(|cue| cue.id == target.cue_id) else {
        return Err(Unplaced::Kept(
            "the section it was for is no longer in the plan".into(),
        ));
    };
    let same = |a: f64, b: f64| (a - b).abs() < 1.0e-6;
    if !same(cue.start_seconds, target.span.0) || !same(cue.end_seconds, target.span.1) {
        return Err(Unplaced::Kept(format!(
            "the section it was for changed while it was made (it ran {} to {}, the section now \
             there runs {} to {})",
            musializer_core::treatment::format_time(target.span.0),
            musializer_core::treatment::format_time(target.span.1),
            musializer_core::treatment::format_time(cue.start_seconds),
            musializer_core::treatment::format_time(cue.end_seconds),
        )));
    }
    Ok(())
}

/// What [`Generations::prepare`] did.
enum Started {
    Job,
    /// The project held the clip; the words say where it now plays.
    Found(String),
}

/// Why [`Generations::prepare`] started nothing.
enum NotStarted {
    /// By name, as any refusal.
    Refused(String),
    /// An earlier send of this request may have been billed: the warning.
    Held(String),
}

impl From<String> for NotStarted {
    fn from(reason: String) -> Self {
        Self::Refused(reason)
    }
}

impl From<&str> for NotStarted {
    fn from(reason: &str) -> Self {
        Self::Refused(reason.to_owned())
    }
}

/// The window's generations: at most one job per section.
pub struct Generations {
    store: Result<Store, String>,
    catalog: Option<Arc<Catalog>>,
    prices: String,
    refresh: Option<std::thread::JoinHandle<(Option<Catalog>, String)>>,
    key: Result<(), String>,
    key_checked: Option<Instant>,
    jobs: Vec<Running>,
    /// By project and section: a cue id of one project names an unrelated
    /// section, or none, in another (the look cards' owner rule).
    last: HashMap<(Owner, u64), LastOutcome>,
    unconfirmed: HashMap<(Owner, u64), UnconfirmedView>,
    /// A treatment is followed this frame.
    followed: bool,
}

impl Default for Generations {
    fn default() -> Self {
        Self::new()
    }
}

impl Generations {
    /// Reads the cached price list; nothing goes on the network.
    #[must_use]
    pub fn new() -> Self {
        let store = Store::from_env();
        let (catalog, prices) = match &store {
            Ok(store) => super::prices(store, false),
            Err(reason) => (None, reason.clone()),
        };
        Self {
            store,
            catalog: catalog.map(Arc::new),
            prices,
            refresh: None,
            key: Err(super::NO_KEY.to_owned()),
            key_checked: None,
            jobs: Vec::new(),
            last: HashMap::new(),
            unconfirmed: HashMap::new(),
            followed: false,
        }
    }

    /// What the card reads for the current project: its sections' jobs,
    /// outcomes and warnings, none of another project's.
    #[must_use]
    pub fn view(&self, workspace: &crate::workspace::Workspace) -> CardView {
        let owner = workspace
            .current_index()
            .and_then(|slot| Owner::of(workspace, slot));
        self.view_for(owner.as_ref())
    }

    fn view_for(&self, owner: Option<&Owner>) -> CardView {
        let mine = |key: &(Owner, u64)| owner == Some(&key.0);
        CardView {
            catalog: self.catalog.clone(),
            prices: self.prices.clone(),
            refreshing: self.refresh.is_some(),
            key: self.key.clone(),
            running: self
                .jobs
                .iter()
                .filter(|running| owner == Some(&running.target.owner))
                .map(|running| {
                    let (stage, elapsed) = running.job.progress();
                    RunningView {
                        cue_id: running.order.cue_id,
                        stage: stage.describe(),
                        elapsed,
                        model: running.order.model.clone(),
                        seconds: running.order.seconds,
                        estimate: running.estimate,
                        cancelling: running.job.is_cancelled(),
                    }
                })
                .collect(),
            last: self
                .last
                .iter()
                .filter(|(key, _)| mine(key))
                .map(|(key, outcome)| (key.1, outcome.clone()))
                .collect(),
            unconfirmed: self
                .unconfirmed
                .iter()
                .filter(|(key, _)| mine(key))
                .map(|(key, held)| (key.1, held.clone()))
                .collect(),
            kept_in: self.store.as_ref().ok().map(super::job::kept_at),
            report: self.describe(owner),
        }
    }

    /// Reads today's price list on a thread: a keyless GET.
    pub fn refresh_prices(&mut self) {
        if self.refresh.is_some() {
            return;
        }
        let Ok(store) = self.store.clone() else {
            return;
        };
        self.refresh = std::thread::Builder::new()
            .name("video-prices".into())
            // Stale or not, a press reads the list again.
            .spawn(move || super::prices_now(&store))
            .ok();
    }

    /// Stops waiting for a section's job, in the current project.
    pub fn cancel(&mut self, cue_id: u64, workspace: &crate::workspace::Workspace) {
        let owner = workspace
            .current_index()
            .and_then(|slot| Owner::of(workspace, slot));
        for running in &self.jobs {
            if running.order.cue_id == cue_id && owner.as_ref() == Some(&running.target.owner) {
                running.job.cancel();
            }
        }
    }

    /// Each frame: the price refresh, the key's presence every two seconds,
    /// whether a treatment is followed, and finished jobs into their tracks.
    pub fn tend(
        &mut self,
        app: &mut crate::App,
        session: Option<&Secret>,
        time_seconds: f64,
        now: f64,
        followed: bool,
    ) {
        self.followed = followed;
        if followed {
            for running in &mut self.jobs {
                running.target.followed = true;
            }
        }
        if self
            .refresh
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(Ok((catalog, line))) =
                self.refresh.take().map(std::thread::JoinHandle::join)
            {
                if catalog.is_some() {
                    self.catalog = catalog.map(Arc::new);
                }
                self.prices = line;
            }
        }
        if self
            .key_checked
            .is_none_or(|checked| checked.elapsed() > Duration::from_secs(2))
        {
            self.key = super::credential(session).map(drop);
            self.key_checked = Some(Instant::now());
        }
        let mut index = 0;
        while index < self.jobs.len() {
            let Some(outcome) = self.jobs[index].job.take_outcome() else {
                index += 1;
                continue;
            };
            let running = self.jobs.remove(index);
            self.finish(app, running, outcome, time_seconds, now);
        }
    }

    fn finish(
        &mut self,
        app: &mut crate::App,
        running: Running,
        outcome: Outcome,
        time_seconds: f64,
        now: f64,
    ) {
        let order = &running.order;
        let estimate = running.estimate.map_or_else(
            || "price unknown".to_owned(),
            |usd| format!("${:.2}", price::ceil_cents(usd)),
        );
        let lost = matches!(
            &outcome,
            Outcome::Failed {
                billing: Billing::Unconfirmed(_),
                ..
            } | Outcome::Stopped {
                billing: Billing::Unconfirmed(_),
            }
        );
        let (severity, title, words) = match outcome {
            Outcome::Done(generated) => {
                let cost = generated
                    .clip
                    .asset
                    .generated
                    .as_ref()
                    .and_then(|record| record.cost_usd);
                let name = generated.clip.asset.name.clone();
                match place(app, &running.target, generated.clip, self.followed, time_seconds, now) {
                    Ok(Placed::Plays(place)) => (
                        Severity::Info,
                        "Clip generated",
                        format!(
                            "{name} plays {place}. {}",
                            match generated.reused {
                                Reuse::New => format!(
                                    "Estimated {estimate}, OpenRouter reported {}.",
                                    cost.map_or_else(|| "no cost".to_owned(), |usd| format!("${usd:.4}"))
                                ),
                                Reuse::Resumed => "It was the job submitted before; nothing new was paid for.".to_owned(),
                                Reuse::Stored => "It was already made; nothing was sent.".to_owned(),
                            }
                        ),
                    ),
                    Ok(Placed::Kept(why)) => (
                        Severity::Warning,
                        "Clip generated, not placed",
                        format!(
                            "{name} is in this project's clip library, not in a section: {why}. \
                             Nothing paid for is lost: choose it in a Video Clip section's clip rows \
                             (a save keeps only clips a section plays), and it stays in {}, where \
                             Generate with the same settings finds it without paying again.",
                            self.store.as_ref().map_or_else(
                                |_| "the store".to_owned(),
                                |store| super::job::kept_at(store).display().to_string()
                            )
                        ),
                    ),
                    Err(reason) => (
                        Severity::Warning,
                        "Clip generated, not placed",
                        format!(
                            "{name}: {reason}. It is kept in {}, and Generate with the same settings \
                             in that project finds it without paying again.",
                            self.store
                                .as_ref()
                                .map_or_else(|_| "the store".to_owned(), |store| super::job::kept_at(store).display().to_string())
                        ),
                    ),
                }
            }
            Outcome::Unconfirmed(intent) => {
                let words = self.hold(
                    (running.target.owner.clone(), order.cue_id),
                    &running.digest,
                    &intent,
                );
                (Severity::Warning, HELD_TITLE, words)
            }
            Outcome::Refused(reason) => (Severity::Warning, "Clip not sent", format!("{reason}.")),
            Outcome::Failed { reason, billing } => (
                Severity::Error,
                "Clip not generated",
                format!(
                    "{}.{}",
                    reason.trim_end_matches('.'),
                    billing.sentence().map(|text| format!(" {text}")).unwrap_or_default()
                ),
            ),
            Outcome::Stopped { billing } => (
                Severity::Warning,
                "Stopped waiting for the clip",
                match billing {
                    Billing::Nothing => "Nothing was sent.".to_owned(),
                    Billing::Unconfirmed(_) => format!(
                        "Stopped while the request was being sent. {}",
                        billing.sentence().unwrap_or_default()
                    ),
                    other => format!(
                        "{} Generate again with the same settings picks the job up instead of paying twice.",
                        other.sentence().unwrap_or_default()
                    ),
                },
            ),
        };
        // A send whose answer was lost: the card warns, and its next Send
        // is the agreement to send it again.
        let key = (running.target.owner.clone(), order.cue_id);
        if lost {
            // The send just lost is the newest; an unreadable store leaves
            // no id, so the next Send is held again with fresh words.
            let attempt = self
                .store
                .as_ref()
                .ok()
                .and_then(|store| store.intent(&running.digest))
                .map(|intent| intent.latest().id.clone())
                .unwrap_or_default();
            self.unconfirmed.insert(
                key.clone(),
                UnconfirmedView {
                    digest: running.digest.clone(),
                    attempt,
                    words: format!("{words} {AGAIN}"),
                },
            );
        }
        app.shell.notify(severity, title, &words);
        self.last.insert(key, LastOutcome { severity, words });
    }

    /// Holds a request whose earlier send may have been billed: the card
    /// shows why, and its next Send agrees to send it again.
    fn hold(&mut self, key: (Owner, u64), digest: &str, intent: &super::store::Intent) -> String {
        let words = match self.store.as_ref() {
            Ok(store) => super::job::unconfirmed_words(intent, store),
            Err(_) => String::new(),
        };
        let words = format!("{words} {AGAIN}");
        self.unconfirmed.insert(
            key,
            UnconfirmedView {
                digest: digest.to_owned(),
                attempt: intent.latest().id.clone(),
                words: words.clone(),
            },
        );
        words
    }

    /// Renders the frames, builds the request and starts the job.
    #[allow(
        clippy::too_many_arguments,
        reason = "the composition root's resources one offline frame needs, as `export_stills`"
    )]
    pub fn start(
        &mut self,
        order: GenerateOrder,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        audio: &RaylibAudio,
        music: &Option<raylib::prelude::Music<'_>>,
        app: &mut crate::App,
        analysis: &mut crate::Analysis,
        renderer: &mut SceneRenderer,
        fonts: &Faces,
        session: Option<&Secret>,
        time_seconds: f64,
        now: f64,
    ) {
        let cue_id = order.cue_id;
        let Some(owner) = Owner::of(&app.workspace, order.track_slot) else {
            return;
        };
        let key = (owner, cue_id);
        match self.prepare(
            order,
            rl,
            thread,
            audio,
            music,
            app,
            analysis,
            renderer,
            fonts,
            session,
            time_seconds,
            now,
        ) {
            Ok(Started::Job) => {
                self.last.remove(&key);
            }
            Ok(Started::Found(words)) => {
                self.unconfirmed.remove(&key);
                app.shell
                    .notify(Severity::Info, "Clip found in the project", &words);
                self.last.insert(
                    key.clone(),
                    LastOutcome {
                        severity: Severity::Info,
                        words,
                    },
                );
            }
            Err(NotStarted::Held(words)) => {
                app.shell.notify(Severity::Warning, HELD_TITLE, &words);
                self.last.insert(
                    key.clone(),
                    LastOutcome {
                        severity: Severity::Warning,
                        words,
                    },
                );
            }
            Err(NotStarted::Refused(reason)) => {
                app.shell
                    .notify(Severity::Warning, "Clip not generated", &reason);
                self.last.insert(
                    key.clone(),
                    LastOutcome {
                        severity: Severity::Warning,
                        words: reason,
                    },
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments, reason = "as `start`")]
    fn prepare(
        &mut self,
        order: GenerateOrder,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        audio: &RaylibAudio,
        music: &Option<raylib::prelude::Music<'_>>,
        app: &mut crate::App,
        analysis: &mut crate::Analysis,
        renderer: &mut SceneRenderer,
        fonts: &Faces,
        session: Option<&Secret>,
        time_seconds: f64,
        now: f64,
    ) -> Result<Started, NotStarted> {
        let owner = Owner::of(&app.workspace, order.track_slot).ok_or("no track is open")?;
        if self
            .jobs
            .iter()
            .any(|running| running.order.cue_id == order.cue_id && running.target.owner == owner)
        {
            return Err("this section is generating already".into());
        }
        let store = self.store.clone()?;
        let secret = super::credential(session)?;
        let endpoint = super::client::Endpoint::from_env()?;
        if app.workspace.current_index() != Some(order.track_slot) {
            return Err("the track it was for is no longer the current one".into());
        }
        if self.followed {
            return Err("a treatment is followed: the file decides what each section plays".into());
        }
        let track = app.workspace.current().ok_or("no track is open")?;
        let cues = track.scene_switches.cues();
        let index = cues
            .iter()
            .position(|cue| cue.id == order.cue_id)
            .ok_or("the section is no longer in the plan")?;
        let start = cues[index].start_seconds;
        let next = cues.get(index + 1).map(|cue| (cue.id, cue.start_seconds));
        let target = Target {
            owner,
            audio_sha256: track.audio_sha256.clone(),
            cue_id: order.cue_id,
            span: (cues[index].start_seconds, cues[index].end_seconds),
            followed: false,
        };
        let output = (track.render_config.width, track.render_config.height);
        let fps = track.render_config.fps;
        let catalog = self.catalog.clone();
        let offer = catalog
            .as_ref()
            .and_then(|catalog| catalog.offer(&order.model))
            .ok_or_else(|| {
                format!(
                    "{} is not on the price list; read today's prices",
                    order.model
                )
            })?;
        let shape = price::shape(offer, output)?;

        // The frames, through the export's still path, at the request's size;
        // the cut into the framed section plain for the moment of the render.
        let (frames_name, folder) = store.private_work("frames")?;
        let mut stills = Vec::new();
        let first_path = folder.join("first.png");
        let last_path = folder.join("last.png");
        if order.first {
            let (_, at) = super::treatment::frames_around(start, fps);
            stills.push((
                super::treatment::frame_middle(at, fps),
                first_path.clone(),
                order.cue_id,
            ));
        }
        if order.last {
            let (next_id, next_start) = next.ok_or("there is no next section to end on")?;
            let (_, at) = super::treatment::frames_around(next_start, fps);
            stills.push((
                super::treatment::frame_middle(at, fps),
                last_path.clone(),
                next_id,
            ));
        }
        if !stills.is_empty() {
            let saved = app
                .workspace
                .current()
                .map(|track| track.scene_switches.clone());
            if let Some(track) = app.workspace.current_mut() {
                for (_, _, cut_into) in &stills {
                    track
                        .scene_switches
                        .set_transition(*cut_into, Transition::CUT);
                }
            }
            let list: Vec<(f64, PathBuf)> = stills
                .iter()
                .map(|(time, path, _)| (*time, path.clone()))
                .collect();
            let rendered = super::treatment::render_frames(
                rl, thread, audio, music, app, analysis, renderer, fonts, &list, shape.size,
            );
            if let (Some(saved), Some(track)) = (saved, app.workspace.current_mut()) {
                track.scene_switches = saved;
            }
            rendered.map_err(|error| format!("its frames could not be rendered: {error}"))?;
        }
        let read =
            |wanted: bool, path: &PathBuf| -> Result<Option<(Vec<u8>, FrameImage)>, String> {
                if !wanted {
                    return Ok(None);
                }
                let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
                Ok(Some((
                    bytes.clone(),
                    FrameImage {
                        sha256: sha256::digest_hex(&bytes),
                        width: shape.size.0,
                        height: shape.size.1,
                    },
                )))
            };
        let first = read(order.first, &first_path)?;
        let last = read(order.last, &last_path)?;
        store.clear_work(&frames_name);
        let mut request = GenerationRequest::new(&offer.id, &order.prompt, order.seconds, &shape);
        request.first = first.as_ref().map(|(_, image)| image.clone());
        request.last = last.as_ref().map(|(_, image)| image.clone());
        request.check(offer).map_err(|error| error.to_string())?;
        let digest = request.digest();
        // The project already holds the clip made for exactly this request
        // (a bundled project on a machine without the store that made it):
        // it is placed, and nothing is sent.
        let held = app.workspace.current().and_then(|track| {
            track
                .clips
                .iter()
                .find(|entry| {
                    entry.unusable.is_none()
                        && entry
                            .asset
                            .generated
                            .as_ref()
                            .is_some_and(|record| record.request_sha256 == digest)
                })
                .cloned()
        });
        if let Some(entry) = held {
            let name = entry.asset.name.clone();
            return match place(app, &target, entry, self.followed, time_seconds, now)? {
                Placed::Plays(place) => Ok(Started::Found(format!(
                    "{name} plays {place}. The project already holds the clip made for exactly \
                     this request, so nothing was sent."
                ))),
                Placed::Kept(why) => Err(NotStarted::Refused(format!(
                    "{name}, the clip made for exactly this request, is in the project already and \
                     was not placed: {why}. Nothing was sent"
                ))),
            };
        }
        let estimate = price::estimate(
            catalog.as_deref(),
            super::now_unix(),
            &offer.id,
            request.seconds,
            &shape,
            request.frame_count(),
        );
        // Consent was given to a price: the request must still carry it.
        let now_usd = estimate.usd().map(price::ceil_cents);
        let shown = order.shown.map(price::ceil_cents);
        if now_usd != shown {
            return Err(NotStarted::Refused(format!(
                "the price changed from {} to {} since the card showed it; look at it again",
                shown.map_or_else(|| "unknown".to_owned(), |usd| format!("${usd:.2}")),
                estimate.short()
            )));
        }
        // Sent before and never answered: held until the person, told it may
        // have been billed, presses Send again for this very request.
        let resend = match store.leftover_intent(&digest) {
            Some(intent) if order.resend.as_deref() == Some(intent.latest().id.as_str()) => {
                Some(intent.latest().id.clone())
            }
            Some(intent) => {
                return Err(NotStarted::Held(self.hold(
                    (target.owner.clone(), order.cue_id),
                    &digest,
                    &intent,
                )))
            }
            None => None,
        };
        self.unconfirmed
            .remove(&(target.owner.clone(), order.cue_id));
        let (poll, deadline, max_bytes) = JobSpec::timings_from_env();
        let job = GenerationJob::spawn(JobSpec {
            name: super::job::clip_name(&request),
            first_png: first.map(|(bytes, _)| bytes),
            last_png: last.map(|(bytes, _)| bytes),
            estimate_usd: estimate.usd(),
            request,
            secret,
            endpoint,
            store,
            poll,
            deadline,
            max_bytes,
            resend,
            ceiling: None,
        });
        self.jobs.push(Running {
            estimate: estimate.usd(),
            order,
            job,
            target,
            digest,
        });
        Ok(Started::Job)
    }

    /// The `generate:` report line: every job of this process (one of
    /// another project marked so), the current project's outcomes.
    #[must_use]
    fn describe(&self, owner: Option<&Owner>) -> String {
        let running: Vec<String> = self
            .jobs
            .iter()
            .map(|running| {
                let (stage, elapsed) = running.job.progress();
                format!(
                    "{}:{}:{:.1}s:{}{}{}",
                    running.order.cue_id,
                    stage_token(&stage),
                    elapsed.as_secs_f64(),
                    running.job.job_id().unwrap_or_else(|| "-".to_owned()),
                    if running.job.is_cancelled() {
                        ":cancelling"
                    } else {
                        ""
                    },
                    if owner == Some(&running.target.owner) {
                        ""
                    } else {
                        ":other-project"
                    }
                )
            })
            .collect();
        let mut last: Vec<String> = self
            .last
            .iter()
            .filter(|(key, _)| owner == Some(&key.0))
            .map(|((_, cue), outcome)| format!("{cue}:{:?}:{}", outcome.severity, outcome.words))
            .collect();
        last.sort();
        format!(
            "prices=\"{}\" key={} running={} last={}",
            self.prices,
            if self.key.is_ok() { "stored" } else { "none" },
            if running.is_empty() {
                "none".to_owned()
            } else {
                running.join(",")
            },
            if last.is_empty() {
                "none".to_owned()
            } else {
                last.join(" | ")
            },
        )
    }
}

fn stage_token(stage: &Stage) -> &'static str {
    match stage {
        Stage::Starting => "starting",
        Stage::Queued => "queued",
        Stage::Submitting => "submitting",
        Stage::Waiting { .. } => "waiting",
        Stage::Downloading => "downloading",
        Stage::Verifying => "verifying",
    }
}

/// Where a finished clip went.
enum Placed {
    /// It plays in its section; where, in words.
    Plays(String),
    /// It is in the project's library, not in a section: why.
    Kept(String),
}

/// Puts a generated clip in the library of the project that asked, and has
/// it play in its section (Video Clip first, if it played another scene)
/// while that is still the section Send was pressed for and no treatment
/// decides the plan. The paid clip is never thrown away: otherwise it stays
/// in the library unplaced, and the words say why.
///
/// # Errors
///
/// The project is not open any more (the clip is only in the store), or it
/// holds as many clips as a project may.
fn place(
    app: &mut crate::App,
    target: &Target,
    entry: crate::clips::TrackClip,
    followed: bool,
    time_seconds: f64,
    now: f64,
) -> Result<Placed, String> {
    let slot = target.owner.slot;
    let current = app.workspace.current_index() == Some(slot);
    let owner = Owner::of(&app.workspace, slot);
    let track = app
        .workspace
        .get_mut(slot)
        .ok_or("the project it was for is no longer open")?;
    let verdict = placement(
        target,
        owner.as_ref(),
        &track.audio_sha256,
        track.scene_switches.cues(),
        followed,
    );
    if verdict == Err(Unplaced::Gone) {
        return Err("the project it was for is no longer open".into());
    }
    let clip = crate::clips::add(&mut track.clips, entry).ok_or_else(|| {
        format!(
            "the project already holds {} clips",
            musializer_core::project::clip::MAX_CLIPS
        )
    })?;
    // In the library for this session; a save writes only what a section
    // plays, so nothing the file holds changes.
    if let Err(Unplaced::Kept(why)) = verdict {
        return Ok(Placed::Kept(why));
    }
    let section = crate::clips::section_target(&track.scene_switches, target.cue_id)
        .ok_or("the section it was for is no longer in the plan")?;
    if let crate::clips::ImportTarget::Section { retarget: true, .. } = section {
        track
            .retarget_scene_cue(target.cue_id, SceneId::Clip)
            .map_err(|error| format!("the section could not become Video Clip: {error}"))?;
    }
    track
        .scene_switches
        .set_clip(target.cue_id, Some(ClipPlacement::new(clip)));
    track.mark_dirty(now);
    if current {
        app.apply_auto_scene_switch(time_seconds);
    }
    Ok(Placed::Plays(crate::clips::import_place(section)))
}

/// The Scenes flyout's Generate…: Look, on the section an import would land
/// in, with its generate block open. With no plan there is no section to
/// generate for, and the notice says so.
pub fn open_block(app: &mut crate::App, time_seconds: f64) {
    let Some(track) = app.workspace.current() else {
        app.shell.notify(
            Severity::Warning,
            "Open a track first",
            "A generated clip plays in a section of a track's video.",
        );
        return;
    };
    let target = crate::clips::import_target(
        &track.scene_switches,
        app.shell.scene_lane.selected_id,
        time_seconds,
    );
    let crate::clips::ImportTarget::Section { cue_id, .. } = target else {
        app.shell.notify(
            Severity::Warning,
            "No section to generate for",
            "A generated clip plays in a section of the plan: turn the plan on (Auto in the plan lane) \
             or split the track into sections first.",
        );
        return;
    };
    app.shell.open_generate_block(cue_id);
}

#[cfg(test)]
mod tests {
    use musializer_core::project::scene_switch::SceneSwitchCue;

    use super::*;

    fn owner() -> Owner {
        Owner {
            slot: 0,
            audio: PathBuf::from("/music/song.wav"),
            project: Some(PathBuf::from("/music/song.musi")),
        }
    }

    fn target() -> Target {
        Target {
            owner: owner(),
            audio_sha256: "a".repeat(64),
            cue_id: 2,
            span: (4.0, 8.0),
            followed: false,
        }
    }

    fn cue(id: u64, start: f64, end: f64) -> SceneSwitchCue {
        SceneSwitchCue {
            id,
            start_seconds: start,
            end_seconds: end,
            ..SceneSwitchCue::default()
        }
    }

    #[test]
    fn the_card_reads_only_the_current_projects_sections() {
        let mut generations = Generations::new();
        let other = Owner {
            project: Some(PathBuf::from("/music/other.musi")),
            ..owner()
        };
        let outcome = LastOutcome {
            severity: Severity::Warning,
            words: "An earlier send may have been billed".into(),
        };
        generations.last.insert((owner(), 2), outcome.clone());
        generations.unconfirmed.insert(
            (owner(), 2),
            UnconfirmedView {
                digest: "d".repeat(64),
                attempt: "1p1".into(),
                words: "held".into(),
            },
        );
        let mine = generations.view_for(Some(&owner()));
        assert_eq!(mine.last.get(&2), Some(&outcome));
        assert!(mine.unconfirmed.contains_key(&2));
        assert!(mine.report.contains("last=2:Warning:"));
        // Project B, section 2: none of A's words, warning or consent.
        let theirs = generations.view_for(Some(&other));
        assert!(theirs.last.is_empty() && theirs.unconfirmed.is_empty());
        assert!(theirs.report.ends_with("last=none"), "{}", theirs.report);
    }

    #[test]
    fn a_clip_is_placed_only_in_the_section_send_was_pressed_for() {
        let audio = "a".repeat(64);
        let plan = [cue(1, 0.0, 4.0), cue(2, 4.0, 8.0), cue(3, 8.0, 12.0)];
        assert_eq!(
            placement(&target(), Some(&owner()), &audio, &plan, false),
            Ok(())
        );
        // A treatment followed now, or at any frame while it was made: the
        // same positional id names the file's section, not the person's.
        let followed = Target {
            followed: true,
            ..target()
        };
        for (target, now) in [(followed, false), (target(), true)] {
            let Err(Unplaced::Kept(why)) = placement(&target, Some(&owner()), &audio, &plan, now)
            else {
                panic!("placed while a treatment decides the plan");
            };
            assert!(why.contains("a treatment was followed"), "{why}");
        }
        // The id names another span: another section.
        let moved = [cue(1, 0.0, 2.0), cue(2, 2.0, 8.0), cue(3, 8.0, 12.0)];
        let Err(Unplaced::Kept(why)) = placement(&target(), Some(&owner()), &audio, &moved, false)
        else {
            panic!("placed in a section that changed");
        };
        assert!(why.contains("changed while it was made"), "{why}");
        assert!(matches!(
            placement(&target(), Some(&owner()), &audio, &plan[..1], false),
            Err(Unplaced::Kept(_))
        ));
        // Another project in the slot, or other audio: only the store has it.
        let other = Owner {
            project: Some(PathBuf::from("/music/other.musi")),
            ..owner()
        };
        assert_eq!(
            placement(&target(), Some(&other), &audio, &plan, false),
            Err(Unplaced::Gone)
        );
        assert_eq!(
            placement(&target(), Some(&owner()), &"b".repeat(64), &plan, false),
            Err(Unplaced::Gone)
        );
        assert_eq!(
            placement(&target(), None, &audio, &plan, false),
            Err(Unplaced::Gone)
        );
    }
}
