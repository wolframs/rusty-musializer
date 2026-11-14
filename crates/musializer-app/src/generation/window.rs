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
    /// The store's folder, for the card's hint.
    pub kept_in: Option<PathBuf>,
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
            kept_in: None,
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
    /// The audio the track that asked has, so a clip never lands in another.
    audio_sha256: String,
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
    last: HashMap<u64, LastOutcome>,
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
        }
    }

    #[must_use]
    pub fn view(&self) -> CardView {
        CardView {
            catalog: self.catalog.clone(),
            prices: self.prices.clone(),
            refreshing: self.refresh.is_some(),
            key: self.key.clone(),
            running: self
                .jobs
                .iter()
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
            last: self.last.clone(),
            kept_in: self.store.as_ref().ok().map(super::job::kept_at),
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

    /// Stops waiting for a section's job.
    pub fn cancel(&mut self, cue_id: u64) {
        for running in &self.jobs {
            if running.order.cue_id == cue_id {
                running.job.cancel();
            }
        }
    }

    /// Whether anything is running (the frame loop keeps tending then).
    #[must_use]
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty() || self.refresh.is_some()
    }

    /// Each frame: the price refresh, the key's presence every two seconds,
    /// and finished jobs into their tracks.
    pub fn tend(&mut self, app: &mut crate::App, session: Option<&Secret>, time_seconds: f64, now: f64) {
        if self.refresh.as_ref().is_some_and(std::thread::JoinHandle::is_finished) {
            if let Some(Ok((catalog, line))) = self.refresh.take().map(std::thread::JoinHandle::join) {
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
        let estimate = running
            .estimate
            .map_or_else(|| "price unknown".to_owned(), |usd| format!("${:.2}", price::ceil_cents(usd)));
        let (severity, title, words) = match outcome {
            Outcome::Done(generated) => {
                let cost = generated
                    .clip
                    .asset
                    .generated
                    .as_ref()
                    .and_then(|record| record.cost_usd);
                let name = generated.clip.asset.name.clone();
                match place(app, order, &running.audio_sha256, generated.clip, time_seconds, now) {
                    Ok(place) => (
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
                    Err(reason) => (
                        Severity::Warning,
                        "Clip generated, not placed",
                        format!(
                            "{name}: {reason}. It is kept, and Generate with the same settings finds it without paying again."
                        ),
                    ),
                }
            }
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
                    other => format!(
                        "{} Generate again with the same settings picks the job up instead of paying twice.",
                        other.sentence().unwrap_or_default()
                    ),
                },
            ),
        };
        app.shell.notify(severity, title, &words);
        self.last.insert(order.cue_id, LastOutcome { severity, words });
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
    ) {
        let cue_id = order.cue_id;
        match self.prepare(order, rl, thread, audio, music, app, analysis, renderer, fonts, session) {
            Ok(()) => {
                self.last.remove(&cue_id);
            }
            Err(reason) => {
                app.shell.notify(Severity::Warning, "Clip not generated", &reason);
                self.last.insert(
                    cue_id,
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
    ) -> Result<(), String> {
        if self.jobs.iter().any(|running| running.order.cue_id == order.cue_id) {
            return Err("this section is generating already".into());
        }
        let store = self.store.clone()?;
        let secret = super::credential(session)?;
        let endpoint = super::client::Endpoint::from_env()?;
        if app.workspace.current_index() != Some(order.track_slot) {
            return Err("the track it was for is no longer the current one".into());
        }
        let track = app.workspace.current().ok_or("no track is open")?;
        let cues = track.scene_switches.cues();
        let index = cues
            .iter()
            .position(|cue| cue.id == order.cue_id)
            .ok_or("the section is no longer in the plan")?;
        let start = cues[index].start_seconds;
        let next = cues.get(index + 1).map(|cue| (cue.id, cue.start_seconds));
        let audio_sha256 = track.audio_sha256.clone();
        let output = (track.render_config.width, track.render_config.height);
        let fps = track.render_config.fps;
        let catalog = self.catalog.clone();
        let offer = catalog
            .as_ref()
            .and_then(|catalog| catalog.offer(&order.model))
            .ok_or_else(|| format!("{} is not on the price list; read today's prices", order.model))?;
        let shape = price::shape(offer, output)?;

        // The frames, through the export's still path, at the request's size;
        // the cut into the framed section plain for the moment of the render.
        let folder = store.work("frames")?;
        let mut stills = Vec::new();
        let first_path = folder.join("first.png");
        let last_path = folder.join("last.png");
        if order.first {
            let (_, at) = super::treatment::frames_around(start, fps);
            stills.push((super::treatment::frame_middle(at, fps), first_path.clone(), order.cue_id));
        }
        if order.last {
            let (next_id, next_start) = next.ok_or("there is no next section to end on")?;
            let (_, at) = super::treatment::frames_around(next_start, fps);
            stills.push((super::treatment::frame_middle(at, fps), last_path.clone(), next_id));
        }
        if !stills.is_empty() {
            let saved = app.workspace.current().map(|track| track.scene_switches.clone());
            if let Some(track) = app.workspace.current_mut() {
                for (_, _, cut_into) in &stills {
                    track.scene_switches.set_transition(*cut_into, Transition::CUT);
                }
            }
            let list: Vec<(f64, PathBuf)> = stills.iter().map(|(time, path, _)| (*time, path.clone())).collect();
            let rendered = super::treatment::render_frames(
                rl, thread, audio, music, app, analysis, renderer, fonts, &list, shape.size,
            );
            if let (Some(saved), Some(track)) = (saved, app.workspace.current_mut()) {
                track.scene_switches = saved;
            }
            rendered.map_err(|error| format!("its frames could not be rendered: {error}"))?;
        }
        let read = |wanted: bool, path: &PathBuf| -> Result<Option<(Vec<u8>, FrameImage)>, String> {
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
        store.clear_work("frames");
        let mut request = GenerationRequest::new(&offer.id, &order.prompt, order.seconds, &shape);
        request.first = first.as_ref().map(|(_, image)| image.clone());
        request.last = last.as_ref().map(|(_, image)| image.clone());
        request.check(offer).map_err(|error| error.to_string())?;
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
            return Err(format!(
                "the price changed from {} to {} since the card showed it; look at it again",
                shown.map_or_else(|| "unknown".to_owned(), |usd| format!("${usd:.2}")),
                estimate.short()
            ));
        }
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
        });
        self.jobs.push(Running {
            estimate: estimate.usd(),
            order,
            job,
            audio_sha256,
        });
        Ok(())
    }

    /// The `generate:` report line.
    #[must_use]
    pub fn describe(&self) -> String {
        let running: Vec<String> = self
            .jobs
            .iter()
            .map(|running| {
                let (stage, elapsed) = running.job.progress();
                format!(
                    "{}:{}:{:.1}s{}",
                    running.order.cue_id,
                    stage_token(&stage),
                    elapsed.as_secs_f64(),
                    if running.job.is_cancelled() { ":cancelling" } else { "" }
                )
            })
            .collect();
        let mut last: Vec<String> = self
            .last
            .iter()
            .map(|(cue, outcome)| format!("{cue}:{:?}", outcome.severity))
            .collect();
        last.sort();
        format!(
            "prices=\"{}\" key={} running={} last={}",
            self.prices,
            if self.key.is_ok() { "stored" } else { "none" },
            if running.is_empty() { "none".to_owned() } else { running.join(",") },
            if last.is_empty() { "none".to_owned() } else { last.join(",") },
        )
    }
}

fn stage_token(stage: &Stage) -> &'static str {
    match stage {
        Stage::Starting => "starting",
        Stage::Submitting => "submitting",
        Stage::Waiting { .. } => "waiting",
        Stage::Downloading => "downloading",
        Stage::Verifying => "verifying",
    }
}

/// Puts a generated clip in the library of the track that asked and has it
/// play in its section (Video Clip first, if it played another scene).
fn place(
    app: &mut crate::App,
    order: &GenerateOrder,
    audio_sha256: &str,
    entry: crate::clips::TrackClip,
    time_seconds: f64,
    now: f64,
) -> Result<String, String> {
    let current = app.workspace.current_index() == Some(order.track_slot);
    let track = app
        .workspace
        .get_mut(order.track_slot)
        .filter(|track| track.audio_sha256 == audio_sha256)
        .ok_or("the track it was for is no longer open")?;
    let target = crate::clips::section_target(&track.scene_switches, order.cue_id)
        .ok_or("the section it was for is no longer in the plan")?;
    let clip = crate::clips::add(&mut track.clips, entry).ok_or_else(|| {
        format!(
            "the project already holds {} clips",
            musializer_core::project::clip::MAX_CLIPS
        )
    })?;
    if let crate::clips::ImportTarget::Section { retarget: true, .. } = target {
        track
            .retarget_scene_cue(order.cue_id, SceneId::Clip)
            .map_err(|error| format!("the section could not become Video Clip: {error}"))?;
    }
    track
        .scene_switches
        .set_clip(order.cue_id, Some(ClipPlacement::new(clip)));
    track.mark_dirty(now);
    if current {
        app.apply_auto_scene_switch(time_seconds);
    }
    Ok(crate::clips::import_place(target))
}
