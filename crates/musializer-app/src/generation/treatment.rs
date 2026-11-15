//! A treatment's generated clips, in a batch run (VG H4).
//!
//! Before the treatment is applied, each section whose clip a model is to
//! make is looked at in time order: its frames are rendered through the
//! export's own still path from a draft of the treatment (the section and any
//! other unmade one standing in, the cut into the framed section plain), the
//! request is built and identified, and the clip it names is looked for — in
//! the open project's library by its provenance, then in the store. A clip
//! that is found costs nothing and needs no flag.
//!
//! **A run never spends by itself.** What is missing is refused by name with
//! its estimate and the total, and the run exits 1, unless it carries
//! `--generate-clips --max-spend USD`.
//!
//! **`--max-spend` is the most this run may newly send** (H4d, 2026-11-15):
//! the sum of the estimates of the requests this run itself submits never
//! passes it, and nothing else counts. A job an earlier run or the window
//! sent is picked up for nothing new, even under `--max-spend 0`. The ceiling
//! is kept where each request leaves ([`Ceiling::take`], under the request's
//! lock, right before its intent is written), so no order of sections and no
//! change another process makes to the store can make the run send more.
//!
//! **The forecast comes before the first request.** Every section is looked
//! at first (found, a job to pick up, sends never answered, new), and the
//! forecast is printed: what each section is expected to do, what earlier
//! runs may already have spent on these requests (reported, never counted),
//! and what this run expects to send. A section whose frames show a generated
//! section this run makes first is looked at again once that one is made;
//! when a fact learnt then changes the forecast, the run says what changed.
//!
//! **A request whose earlier sends lost their answers** (an intent left in
//! the store, H4b) may have been billed for each of them. It is refused by
//! name unless the run also carries `--resend-unconfirmed`; then the resend
//! is a new send of this run and counts at its estimate, once.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use musializer_core::generation::catalog::Catalog;
use musializer_core::generation::price::{self, Price};
use musializer_core::generation::request::{FrameImage, GenerationRequest};
use musializer_core::project::sha256;
use musializer_core::treatment::{ClipGeneration, FrameSource, Treatment};
use musializer_runtime::font::Faces;
use raylib::prelude::{RaylibAudio, RaylibHandle, RaylibThread};

use super::job::{Ceiling, GenerationJob, JobSpec, Outcome, Reuse};
use super::store::Store;
use crate::clips::TrackClip;
use crate::scene_host::SceneRenderer;

/// The largest frame image file a treatment may name.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// What a run may spend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spending {
    /// `--generate-clips` with `--max-spend USD`.
    pub ceiling_usd: Option<f64>,
    /// `--max-spend` as typed: the ceiling is printed as given (H4d).
    pub ceiling_typed: Option<String>,
    /// `--resend-unconfirmed`: a request sent before whose answer never
    /// arrived may be sent again (it may then be paid twice).
    pub resend_unconfirmed: bool,
}

/// The command line's `--resolution`, `--fps` and `--quality`, which the
/// export applies after the treatment's `output` block. The frames sent are
/// the export's frames at those cuts, so they are rendered under the same
/// overrides (size and aspect follow `--resolution`; the seed is the
/// treatment's own, with no flag).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Overrides {
    pub resolution: Option<(u32, u32)>,
    pub fps: Option<u32>,
    pub quality: Option<musializer_core::timing::render_export::Quality>,
}

/// A treatment whose generated clips are attached, with the library entries
/// they need.
pub struct Prepared {
    pub treatment: Treatment,
    pub entries: Vec<(String, TrackClip)>,
}

/// The sentence a treatment applied without this pass gets for a generated
/// section (a window following the file, a session launch).
///
/// # Errors
///
/// The first generated section whose clip is not attached.
pub fn refuse_unprepared(treatment: &Treatment) -> Result<(), String> {
    match treatment
        .clip_generations()
        .into_iter()
        .find(|generation| !generation.attached)
    {
        None => Ok(()),
        Some(generation) => Err(format!(
            "{}: a generated clip is found or made by a batch run, which renders its frames \
             first: `musializer --treatment FILE --save-project FILE.musi` shows what it would \
             cost, `--generate-clips --max-spend USD` makes it; open the project it saves",
            generation.place
        )),
    }
}

/// The frame index a still at `seconds` lands on is `floor(seconds * fps)`;
/// these are the middle of the frame, inside it on both sides.
pub(crate) fn frame_middle(index: u64, fps: u32) -> f64 {
    (index as f64 + 0.5) / f64::from(fps)
}

/// The last frame strictly before `at`, and the first frame at or after it.
pub(crate) fn frames_around(at: f64, fps: u32) -> (Option<u64>, u64) {
    let first_at = (at * f64::from(fps) - 1.0e-9).ceil().max(0.0) as u64;
    (first_at.checked_sub(1), first_at)
}

/// One generated section, resolved or refused.
#[derive(Debug)]
enum Resolution {
    Found,
    Generated {
        estimate: Option<f64>,
        cost: Option<f64>,
        resumed: bool,
    },
    Missing {
        estimate: Price,
        why: String,
    },
}

/// A generated section's request, built from its rendered frames.
struct Ready {
    request: GenerationRequest,
    digest: String,
    estimate: Price,
    first_png: Option<Vec<u8>>,
    last_png: Option<Vec<u8>>,
}

/// What is known of a request before anything is sent (H4c).
enum State {
    /// Nothing of it was sent before.
    New,
    /// A job submitted before and not downloaded, or one another window or
    /// run is sending now: picked up, which costs this run nothing. Its
    /// estimate, when known, is what an earlier send may have spent.
    PickUp(Option<f64>),
    /// Sent before and never answered: each send may have been billed.
    Held(super::store::Intent),
}

/// Where a section stands when it is looked at.
enum Look {
    Found(Box<TrackClip>),
    Ready(Box<Ready>, State),
    /// Its frames show a generated section this run has yet to make, so its
    /// request is known only once that one is made.
    Waits {
        on: usize,
        on_place: String,
        estimate: Price,
    },
    Refused {
        estimate: Price,
        why: String,
    },
}

/// What the forecast expects of a section the run will try (H4d). The
/// ceiling itself is kept where each request leaves ([`Ceiling::take`]); this
/// is what the person reads before the first one does.
enum Forecast {
    /// A job sent before, picked up: nothing new is sent.
    PickUp,
    /// A new send of this run, at its estimate.
    Send(f64),
    /// Sent again after this many unanswered sends (`--resend-unconfirmed`):
    /// a new send of this run, at its estimate.
    Again(f64, usize),
    /// Its request is known only once the named section is made; counted at
    /// its estimate meanwhile.
    Waits(f64, String),
    /// Expected not to be sent, and why. It is tried all the same: what is
    /// sent before it may come to less than forecast (a request another run
    /// sent meanwhile is picked up for nothing).
    Over(String),
}

/// What the run does with a section.
enum Step {
    /// Found, or refused before anything is tried.
    Settled(Option<String>),
    /// Tried in time order, as forecast.
    Try(Forecast),
}

/// One section in the plan.
struct Entry {
    generation: ClipGeneration,
    look: Option<Look>,
    /// How many clips were attached when it was looked at: one attached
    /// since may show in its frames, so it is looked at again.
    seen: usize,
    step: Step,
}

/// What rendering a section's frames needs: the composition root's
/// resources, as `export_stills`.
struct Frames<'r, 'audio> {
    path: &'r Path,
    overrides: Overrides,
    rl: &'r mut RaylibHandle,
    thread: &'r RaylibThread,
    audio: &'audio RaylibAudio,
    music: &'r mut Option<raylib::prelude::Music<'audio>>,
    analysis: &'r mut crate::Analysis,
    app: &'r mut crate::App,
    scratch: &'r mut [f32],
    renderer: &'r mut SceneRenderer,
    fonts: &'r Faces,
    catalog: Option<&'r Catalog>,
    prices_line: &'r str,
    frames_dir: &'r Path,
    store: &'r Store,
}

impl Frames<'_, '_> {
    /// Renders the section's frames from a draft (every unmade generated
    /// section standing in), builds its request and reads what the project
    /// and the store hold for it. Sends nothing. `blocked`: generated
    /// sections this run will not make.
    fn look(
        &mut self,
        generation: &ClipGeneration,
        treatment: &Treatment,
        entries: &[(String, TrackClip)],
        blocked: &[usize],
    ) -> Result<Look, String> {
        let resolution_override = self.overrides.resolution;
        let unmade = |segment: usize| {
            treatment
                .clip_generations()
                .into_iter()
                .find(|other| other.segment == segment && !other.attached)
        };
        // A section whose previous one is a generated section not made has
        // no frame to start on yet.
        if generation.spec.first == Some(FrameSource::Previous) {
            if let Some(before) = generation.segment.checked_sub(1).and_then(unmade) {
                let estimate = estimate_without_frames(
                    generation,
                    self.catalog,
                    self.app,
                    resolution_override,
                );
                if blocked.contains(&before.segment) {
                    return Ok(Look::Refused {
                        estimate,
                        why: "its first frame is the section before's, which is missing".into(),
                    });
                }
                return Ok(Look::Waits {
                    on: before.segment,
                    on_place: before.place,
                    estimate,
                });
            }
        }

        // The frames, from a draft: every unmade section standing in.
        let mut draft = treatment.clone();
        for pending in treatment.clip_generations() {
            if !pending.attached {
                draft.stand_in(pending.segment);
            }
        }
        if generation.spec.first == Some(FrameSource::Previous) {
            draft.cut_into(generation.segment);
        }
        if generation.spec.last == Some(FrameSource::Next) {
            draft.cut_into(generation.segment + 1);
        }
        crate::apply_treatment_prepared(
            self.path,
            Some(Prepared {
                treatment: draft,
                entries: entries.to_vec(),
            }),
            self.audio,
            self.analysis,
            self.music,
            self.app,
            self.scratch,
            false,
            0.0,
        )
        .map_err(|error| format!("{}: the draft for its frames: {error}", generation.place))?;
        // The export applies the command line's output flags after the
        // treatment's; so do the frames, or a 24 fps treatment rendered at
        // 30 sends the picture at 95/24 s for a cut the export shows at 119/30.
        if let Some(track) = self.app.workspace.current_mut() {
            if let Some((width, height)) = self.overrides.resolution {
                track.render_config.width = width;
                track.render_config.height = height;
            }
            if let Some(fps) = self.overrides.fps {
                track.render_config.fps = fps;
            }
            if let Some(quality) = self.overrides.quality {
                track.render_config.set_quality(quality);
            }
        }
        let (output, fps) = {
            let track = self
                .app
                .workspace
                .current()
                .ok_or("a treatment directs a track: name one in its audio block")?;
            (
                resolution_override
                    .unwrap_or((track.render_config.width, track.render_config.height)),
                track.render_config.fps,
            )
        };
        let Some(offer) = self
            .catalog
            .and_then(|catalog| catalog.offer(&generation.spec.model))
        else {
            return Err(format!(
                "{}: {} is not on the price list ({}); `--vocabulary` lists the models",
                generation.place, generation.spec.model, self.prices_line
            ));
        };
        let shape = price::shape(offer, output)
            .map_err(|error| format!("{}: {error}", generation.place))?;

        let mut stills: Vec<(f64, PathBuf)> = Vec::new();
        let mut first = None;
        let mut last = None;
        match &generation.spec.first {
            Some(FrameSource::Previous) => {
                let (before, _) = frames_around(generation.at, fps);
                let index = before
                    .ok_or_else(|| format!("{}.first: no frame before 0:00", generation.place))?;
                let file = self
                    .frames_dir
                    .join(format!("{}-first.png", generation.segment));
                stills.push((frame_middle(index, fps), file.clone()));
                first = Some(file);
            }
            Some(FrameSource::File(name)) => first = Some(resolve(self.path, name)),
            Some(FrameSource::Next) | None => {}
        }
        match &generation.spec.last {
            Some(FrameSource::Next) => {
                let next_at = generation
                    .next_at
                    .ok_or_else(|| format!("{}.last: no section after it", generation.place))?;
                let (_, index) = frames_around(next_at, fps);
                let file = self
                    .frames_dir
                    .join(format!("{}-last.png", generation.segment));
                stills.push((frame_middle(index, fps), file.clone()));
                last = Some(file);
            }
            Some(FrameSource::File(name)) => last = Some(resolve(self.path, name)),
            Some(FrameSource::Previous) | None => {}
        }
        // A frame inside a transition shows both sides of its cut. A side that
        // is a generated section not made yet is a stand-in, and a stand-in
        // is never sent: the section waits for that one, or, when this run
        // will not make it, is refused (a section shorter than the blend into
        // it, after a generated one this run could not make).
        if let Some((segment, place, why)) = stand_in_in_frame(&stills, self.app, treatment) {
            let estimate =
                estimate_without_frames(generation, self.catalog, self.app, resolution_override);
            if blocked.contains(&segment) {
                return Ok(Look::Refused { estimate, why });
            }
            return Ok(Look::Waits {
                on: segment,
                on_place: place,
                estimate,
            });
        }
        if !stills.is_empty() {
            render_frames(
                self.rl,
                self.thread,
                self.audio,
                self.music,
                self.app,
                self.analysis,
                self.renderer,
                self.fonts,
                &stills,
                shape.size,
            )
            .map_err(|error| format!("{}: its frames: {error}", generation.place))?;
        }
        let read_frame = |file: &Option<PathBuf>| -> Result<Option<(Vec<u8>, FrameImage)>, String> {
            let Some(file) = file else { return Ok(None) };
            let bytes = read_image(file)
                .map_err(|error| format!("{}: {}: {error}", generation.place, file.display()))?;
            let size = png_size(&bytes).unwrap_or((0, 0));
            Ok(Some((
                bytes.clone(),
                FrameImage {
                    sha256: sha256::digest_hex(&bytes),
                    width: size.0,
                    height: size.1,
                },
            )))
        };
        let first = read_frame(&first)?;
        let last = read_frame(&last)?;
        let mut request = GenerationRequest::new(
            &offer.id,
            &generation.spec.prompt,
            generation.spec.seconds,
            &shape,
        );
        request.first = first.as_ref().map(|(_, image)| image.clone());
        request.last = last.as_ref().map(|(_, image)| image.clone());
        request
            .check(offer)
            .map_err(|error| format!("{}: {error}", generation.place))?;
        let digest = request.digest();
        let estimate = price::estimate(
            self.catalog,
            super::now_unix(),
            &offer.id,
            request.seconds,
            &shape,
            request.frame_count(),
        );

        // Found: in the open project, or in the store.
        let in_project = self.app.workspace.current().and_then(|track| {
            track.clips.iter().find(|entry| {
                entry.unusable.is_none()
                    && entry
                        .asset
                        .generated
                        .as_ref()
                        .is_some_and(|record| record.request_sha256 == digest)
            })
        });
        let found = match in_project {
            Some(entry) => Some((entry.clone(), "the project")),
            None => self.store.find(&digest).and_then(|(file, record)| {
                let mut entry = crate::clips::import(&file).ok()?;
                entry.asset.name = super::job::clip_name(&request);
                entry.asset.generated = Some(record);
                Some((entry, "the store"))
            }),
        };
        if let Some((entry, from)) = found {
            println!(
                "generate:        {} found in {from}, nothing sent (request {})",
                generation.place,
                &digest[..12]
            );
            return Ok(Look::Found(Box::new(entry)));
        }
        // What the store says of it, read before anything is sent: a job
        // submitted before or in flight now is picked up, for nothing new;
        // sends never answered are held. Both are said, neither is counted
        // against this run's ceiling (H4d).
        let state = if let Some(record) = self.store.job(&digest) {
            State::PickUp(record.estimate_usd)
        } else if self.store.in_flight(&digest) {
            State::PickUp(estimate.usd())
        } else {
            match self.store.leftover_intent(&digest) {
                Ok(Some(intent)) => match super::job::too_many_words(&intent, self.store) {
                    Some(why) => return Ok(Look::Refused { estimate, why }),
                    None => State::Held(intent),
                },
                Ok(None) => State::New,
                // A record that cannot be read holds the request (H4d).
                Err(unreadable) => {
                    return Ok(Look::Refused {
                        estimate,
                        why: unreadable.words(),
                    })
                }
            }
        };
        println!(
            "generate:        {} missing: {} s of {} at {} {}, {}{} — {}",
            generation.place,
            request.seconds,
            offer.id,
            shape.resolution,
            shape.aspect_ratio,
            estimate.short(),
            if matches!(state, State::PickUp(_)) {
                " (submitted before, or being sent by another window or run: it is picked up, \
                 not sent again, which costs this run nothing)"
            } else {
                ""
            },
            request.leaves()
        );
        if let State::Held(intent) = &state {
            println!(
                "generate:        {} {}",
                generation.place,
                super::job::unconfirmed_words(intent, self.store)
            );
        }
        Ok(Look::Ready(
            Box::new(Ready {
                digest,
                estimate,
                first_png: first.map(|(bytes, _)| bytes),
                last_png: last.map(|(bytes, _)| bytes),
                request,
            }),
            state,
        ))
    }
}

/// "$0.12", rounded up to the cent.
fn cents(usd: f64) -> String {
    format!("${:.2}", price::ceil_cents(usd))
}

/// What a held request's refusal adds after the warning: how to send it
/// again, and what that costs this run.
fn resend_words(estimate: &Price) -> String {
    format!(
        "This run does not send it again: add --resend-unconfirmed for a new send, which counts \
         against --max-spend at its estimate ({})",
        estimate.short()
    )
}

/// Resolves every generated section of the treatment at `path`.
///
/// Returns `None` when it has none. Every section is looked at first — its
/// frames rendered, its request built, and what the project and the store
/// hold for it read — and, under `--max-spend`, the forecast is printed
/// before anything leaves (H4d): what each section is expected to do, what
/// earlier runs may already have spent on these requests (reported, not
/// counted), and what this run expects to send. Then the sections are tried
/// in time order; the ceiling is kept by [`Ceiling::take`] as each request
/// leaves, and when a fact learnt later changes the forecast the run says so.
///
/// # Errors
///
/// A section that cannot be resolved, or missing clips the run may not (or
/// could not) pay for — after every section has been tried, so the refusal
/// lists them all with the total.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the composition root's resources one offline frame needs, as `export_stills`; \
              the look, the forecast and the sends read best in one place"
)]
pub fn prepare<'audio>(
    path: &Path,
    spending: Spending,
    session_key: Option<&musializer_core::assist::secret::Secret>,
    overrides: Overrides,
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    audio: &'audio RaylibAudio,
    music: &mut Option<raylib::prelude::Music<'audio>>,
    analysis: &mut crate::Analysis,
    app: &mut crate::App,
    scratch: &mut [f32],
    renderer: &mut SceneRenderer,
    fonts: &Faces,
) -> Result<Option<Prepared>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut treatment = Treatment::parse(&bytes).map_err(|error| error.to_string())?;
    if treatment.clip_generations().is_empty() {
        return Ok(None);
    }
    let store = Store::from_env()?;
    let (catalog, prices_line) = super::prices(&store, true);
    println!(
        "generate:        {prices_line}; clips kept in {}",
        store.root().display()
    );
    let ceiling = spending.ceiling_usd.map(|usd| {
        let typed = spending
            .ceiling_typed
            .clone()
            .unwrap_or_else(|| ceiling_words(usd));
        Arc::new(Ceiling::new(usd, &typed))
    });
    let (frames_name, frames_dir) = store.private_work("frames")?;
    let mut frames = Frames {
        path,
        overrides,
        rl,
        thread,
        audio,
        music,
        analysis,
        app,
        scratch,
        renderer,
        fonts,
        catalog: catalog.as_ref(),
        prices_line: &prices_line,
        frames_dir: &frames_dir,
        store: &store,
    };
    let mut entries: Vec<(String, TrackClip)> = Vec::new();

    // 1. Look at every section; send nothing.
    let mut plan: Vec<Entry> = Vec::new();
    for generation in treatment.clip_generations() {
        let look = frames.look(&generation, &treatment, &entries, &[])?;
        if let Look::Found(entry) = &look {
            attach(&mut treatment, &mut entries, &generation, (**entry).clone())?;
        }
        plan.push(Entry {
            generation,
            look: Some(look),
            seen: entries.len(),
            step: Step::Settled(None),
        });
    }

    // 2. The forecast, in time order. What earlier runs may have spent on
    // these requests is added up only to be said: it is not this run's.
    let mut before = (0.0f64, 0usize, 0usize, 0usize);
    let mut forecast_sent = 0.0f64;
    let mut refused: Vec<usize> = Vec::new();
    for entry in &mut plan {
        let place = entry.generation.place.clone();
        let (estimate, state, waits_on) = match &entry.look {
            Some(Look::Found(..)) | None => continue,
            Some(Look::Refused { why, .. }) => {
                println!("generate:        {place} refused: {why}");
                refused.push(entry.generation.segment);
                entry.step = Step::Settled(Some(why.clone()));
                continue;
            }
            Some(Look::Ready(ready, state)) => (ready.estimate.clone(), Some(state), None),
            Some(Look::Waits {
                on,
                on_place,
                estimate,
            }) => (estimate.clone(), None, Some((*on, on_place.clone()))),
        };
        match state {
            Some(State::PickUp(usd)) => {
                before.0 += usd.unwrap_or(0.0);
                before.1 += 1;
                before.2 += 1;
            }
            Some(State::Held(intent)) => {
                before.0 += intent.total_usd().unwrap_or(0.0);
                before.1 += intent.attempts.len();
                before.3 += intent.attempts.len();
            }
            _ => {}
        }
        let words = match state {
            Some(State::Held(intent)) => Some(super::job::unconfirmed_words(intent, &store)),
            _ => None,
        };
        let Some(ceiling) = &ceiling else {
            // No flag: nothing is sent, and nothing is fetched either.
            let why = match (state, &words) {
                (Some(State::PickUp(_)), _) => {
                    "the run has no --generate-clips --max-spend; a job \
                    sent before for it is waiting, and --generate-clips --max-spend 0 fetches it \
                    without sending anything new"
                        .to_owned()
                }
                (_, Some(words)) => format!("the run has no --generate-clips --max-spend. {words}"),
                _ => "the run has no --generate-clips --max-spend".to_owned(),
            };
            entry.step = Step::Settled(Some(why));
            continue;
        };
        if let Some((on, on_place)) = &waits_on {
            if refused.contains(on) {
                let why = format!("it waits for {on_place}, which this run will not make");
                println!("generate:        {place} refused: {why}");
                refused.push(entry.generation.segment);
                entry.step = Step::Settled(Some(why));
                continue;
            }
        }
        let attempts = match state {
            Some(State::PickUp(_)) => {
                entry.step = Step::Try(Forecast::PickUp);
                continue;
            }
            Some(State::Held(_)) if !spending.resend_unconfirmed => {
                let why = format!(
                    "{} {}",
                    words.as_deref().unwrap_or_default(),
                    resend_words(&estimate)
                );
                // The warning itself was printed when the section was looked at.
                println!(
                    "generate:        {place} refused: {}",
                    resend_words(&estimate)
                );
                refused.push(entry.generation.segment);
                entry.step = Step::Settled(Some(why));
                continue;
            }
            Some(State::Held(intent)) => Some(intent.attempts.len()),
            _ => None,
        };
        entry.step = Step::Try(match estimate.usd() {
            None => Forecast::Over(format!(
                "{}; a run with --max-spend sends only what it can count",
                match &estimate {
                    Price::Unknown(reason) => reason.reason(),
                    Price::Known(_) => String::new(),
                }
            )),
            Some(usd) if forecast_sent + usd > ceiling.limit_usd() + 1.0e-9 => {
                Forecast::Over(ceiling.past(forecast_sent, usd, true))
            }
            Some(usd) => {
                forecast_sent += usd;
                match (attempts, waits_on) {
                    (Some(attempts), _) => Forecast::Again(usd, attempts),
                    (None, Some((_, on_place))) => Forecast::Waits(usd, on_place),
                    (None, None) => Forecast::Send(usd),
                }
            }
        });
    }
    if let Some(ceiling) = &ceiling {
        let (mut sends, mut pickups) = (0usize, 0usize);
        for entry in &plan {
            let place = &entry.generation.place;
            match &entry.step {
                Step::Try(Forecast::PickUp) => {
                    pickups += 1;
                    println!(
                        "generate:        plan: {place} picks up the job sent before; nothing new is sent"
                    );
                }
                Step::Try(Forecast::Send(usd)) => {
                    sends += 1;
                    println!("generate:        plan: {place} is sent, {}", cents(*usd));
                }
                Step::Try(Forecast::Again(usd, attempts)) => {
                    sends += 1;
                    println!(
                        "generate:        plan: {place} is sent again after {attempts} unanswered \
                         send{}: a new send of this run, {}",
                        if *attempts == 1 { "" } else { "s" },
                        cents(*usd)
                    );
                }
                Step::Try(Forecast::Waits(usd, on_place)) => {
                    sends += 1;
                    println!(
                        "generate:        plan: {place} waits for {on_place} to be made; its \
                         request is known then, counted at {} until it is",
                        cents(*usd)
                    );
                }
                Step::Try(Forecast::Over(why)) => {
                    println!("generate:        plan: {place} would not be sent: {why}");
                }
                Step::Settled(_) => {}
            }
        }
        if before.1 > 0 {
            let mut what = Vec::new();
            if before.2 > 0 {
                what.push(format!(
                    "{} job{} to pick up",
                    before.2,
                    if before.2 == 1 { "" } else { "s" }
                ));
            }
            if before.3 > 0 {
                what.push(format!(
                    "{} send{} never answered",
                    before.3,
                    if before.3 == 1 { "" } else { "s" }
                ));
            }
            println!(
                "generate:        before this run: about {} may already have been billed for {} \
                 earlier send{} of these requests ({}); this is not part of --max-spend. Every send \
                 is in {}",
                cents(before.0),
                before.1,
                if before.1 == 1 { "" } else { "s" },
                what.join(", "),
                store.root().join("spending.tsv").display()
            );
        }
        println!(
            "generate:        plan: this run would send {sends} request{}, about {} of its {} \
             allowed{}; nothing has been sent yet",
            if sends == 1 { "" } else { "s" },
            cents(forecast_sent),
            ceiling.shown(),
            if pickups > 0 {
                format!(
                    ", and pick up {pickups} job{}",
                    if pickups == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            }
        );
    }

    // 3. Try, in time order, what the forecast did not settle. Each send is
    // counted against the ceiling where it leaves, under its request's lock.
    let mut outcomes: Vec<(ClipGeneration, Resolution)> = Vec::new();
    let mut blocked: Vec<usize> = refused.clone();
    for index in 0..plan.len() {
        let entry = &mut plan[index];
        let generation = entry.generation.clone();
        let forecast = match std::mem::replace(&mut entry.step, Step::Settled(None)) {
            Step::Settled(None) => {
                outcomes.push((generation, Resolution::Found));
                continue;
            }
            Step::Settled(Some(why)) => {
                let estimate = match &entry.look {
                    Some(Look::Ready(ready, _)) => ready.estimate.clone(),
                    Some(Look::Waits { estimate, .. } | Look::Refused { estimate, .. }) => {
                        estimate.clone()
                    }
                    _ => Price::Unknown(price::UnknownPrice::NotListed),
                };
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
                continue;
            }
            Step::Try(forecast) => forecast,
        };
        let place = generation.place.clone();
        // Looked at again when a clip attached since may show in its frames,
        // or when it waited for one.
        let stale = entry.seen != entries.len() || matches!(entry.look, Some(Look::Waits { .. }));
        let (planned_digest, planned_pickup, planned_held) = match &entry.look {
            Some(Look::Ready(ready, state)) => (
                Some(ready.digest.clone()),
                matches!(state, State::PickUp(_)),
                matches!(state, State::Held(_)),
            ),
            _ => (None, false, false),
        };
        let look = if stale {
            frames.look(&generation, &treatment, &entries, &blocked)?
        } else {
            entry.look.take().expect("a planned section was looked at")
        };
        let (ready, state) = match look {
            Look::Found(found) => {
                println!("generate:        {place}: found now, made since the plan; nothing sent");
                attach(&mut treatment, &mut entries, &generation, *found)?;
                outcomes.push((generation, Resolution::Found));
                continue;
            }
            Look::Refused { estimate, why } => {
                println!("generate:        {place} refused: {why}");
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
                continue;
            }
            Look::Waits {
                on_place, estimate, ..
            } => {
                let why = format!("it waits for {on_place}, which was not made");
                println!("generate:        {place} refused: {why}");
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
                continue;
            }
            Look::Ready(ready, state) => (ready, state),
        };
        if stale && planned_digest.as_deref() != Some(ready.digest.as_str()) {
            println!(
                "generate:        {place} looked at again now that the sections before it are made"
            );
        }
        // What the store says now, where it differs from the forecast.
        let changed = match (&state, planned_pickup, planned_held) {
            (State::PickUp(_), true, _) | (State::Held(_), _, true) => None,
            (State::PickUp(_), false, _) => Some(
                "now a job to pick up: it was sent since the plan by another window or run, and \
                 picking it up costs this run nothing"
                    .to_owned(),
            ),
            (_, true, _) => Some(
                "the job the plan was to pick up is no longer there (it ended without a clip), so \
                 this is a new send of this run"
                    .to_owned(),
            ),
            (State::Held(_), _, false) => Some(
                "the plan counted it as a new send; its request, known now, was sent before and \
                 never answered"
                    .to_owned(),
            ),
            _ => None,
        };
        if let Some(changed) = &changed {
            println!("generate:        {place}: {changed}");
        }
        if let State::Held(intent) = &state {
            if !spending.resend_unconfirmed {
                let why = format!(
                    "{} {}",
                    super::job::unconfirmed_words(intent, &store),
                    resend_words(&ready.estimate)
                );
                println!(
                    "generate:        {place} refused: {}",
                    resend_words(&ready.estimate)
                );
                blocked.push(generation.segment);
                outcomes.push((
                    generation,
                    Resolution::Missing {
                        estimate: ready.estimate.clone(),
                        why,
                    },
                ));
                continue;
            }
        }
        let secret = super::credential(session_key).map_err(|error| format!("{place}: {error}"))?;
        let endpoint = super::client::Endpoint::from_env()?;
        let (poll, deadline, max_bytes) = JobSpec::timings_from_env();
        let Ready {
            request,
            estimate,
            first_png,
            last_png,
            ..
        } = *ready;
        let job = GenerationJob::spawn(JobSpec {
            name: super::job::clip_name(&request),
            first_png,
            last_png,
            estimate_usd: estimate.usd(),
            request,
            secret,
            endpoint,
            store: store.clone(),
            poll,
            deadline,
            max_bytes,
            resend: match &state {
                State::Held(intent) => Some(intent.latest().id.clone()),
                _ => None,
            },
            ceiling: ceiling.clone(),
        });
        let outcome = job.wait(|stage, elapsed| {
            println!(
                "generate:        {place} {} ({:.0} s)",
                stage.describe(),
                elapsed.as_secs_f64()
            );
        });
        match outcome {
            Outcome::Done(generated) => {
                let record = generated.clip.asset.generated.clone();
                let cost = record.as_ref().and_then(|record| record.cost_usd);
                println!(
                    "generate:        {place} {}: {} ({}x{}, {:.2} s), estimate {}, reported {}",
                    match generated.reused {
                        Reuse::New => "generated",
                        Reuse::Resumed => "picked up",
                        Reuse::Stored => "found",
                    },
                    generated.clip.asset.name,
                    generated.clip.asset.width,
                    generated.clip.asset.height,
                    generated.clip.asset.duration_seconds,
                    estimate.short(),
                    cost.map_or_else(|| "nothing".to_owned(), |usd| format!("${usd:.4}")),
                );
                if let Forecast::Over(why) = &forecast {
                    println!(
                        "generate:        {place}: the plan expected it not to be sent ({why}); \
                         what this run sent before it came to less than forecast"
                    );
                }
                let resumed = generated.reused != Reuse::New;
                attach(&mut treatment, &mut entries, &generation, generated.clip)?;
                outcomes.push((
                    generation,
                    Resolution::Generated {
                        estimate: estimate.usd(),
                        cost,
                        resumed,
                    },
                ));
            }
            Outcome::Refused(why) => {
                println!("generate:        {place} refused: {why}");
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
            }
            Outcome::Failed { reason, billing } => {
                let why = match billing.sentence() {
                    Some(text) => format!("{}. {text}", reason.trim_end_matches('.')),
                    None => reason,
                };
                println!("generate:        {place} failed: {why}");
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
            }
            Outcome::Unconfirmed(intent) => {
                // Another caller's send was lost since this run looked:
                // nothing was sent now.
                let why = format!(
                    "{} Nothing was sent now; add --resend-unconfirmed to send it again",
                    super::job::unconfirmed_words(&intent, &store)
                );
                println!("generate:        {place} refused: {why}");
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
            }
            Outcome::Stopped { billing } => {
                let why = format!(
                    "stopped{}",
                    billing
                        .sentence()
                        .map(|text| format!(". {text}"))
                        .unwrap_or_default()
                );
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
            }
        }
    }
    store.clear_work(&frames_name);
    if let Some(ceiling) = &ceiling {
        let (usd, count) = ceiling.sent();
        println!(
            "generate:        this run sent {count} request{}, about {} of its {} allowed",
            if count == 1 { "" } else { "s" },
            cents(usd),
            ceiling.shown()
        );
    }

    // The summary, and the refusal when anything is missing.
    let missing: Vec<&(ClipGeneration, Resolution)> = outcomes
        .iter()
        .filter(|(_, resolution)| matches!(resolution, Resolution::Missing { .. }))
        .collect();
    let spent: f64 = outcomes
        .iter()
        .filter_map(|(_, resolution)| match resolution {
            Resolution::Generated {
                cost,
                estimate,
                resumed: false,
            } => cost.or(*estimate),
            _ => None,
        })
        .sum();
    let found = outcomes
        .iter()
        .filter(|(_, resolution)| {
            matches!(
                resolution,
                Resolution::Found | Resolution::Generated { resumed: true, .. }
            )
        })
        .count();
    let made = outcomes
        .iter()
        .filter(|(_, resolution)| {
            matches!(resolution, Resolution::Generated { resumed: false, .. })
        })
        .count();
    if missing.is_empty() {
        println!(
            "generate:        {} generated sections: {found} found, {made} generated now (${:.2} reported or estimated)",
            outcomes.len(),
            price::ceil_cents(spent.max(0.0))
        );
        return Ok(Some(Prepared { treatment, entries }));
    }
    let mut total = 0.0;
    let mut unknown = 0;
    for (_, resolution) in &missing {
        if let Resolution::Missing { estimate, .. } = resolution {
            match estimate.usd() {
                Some(usd) => total += usd,
                None => unknown += 1,
            }
        }
    }
    let list = missing
        .iter()
        .map(|(generation, resolution)| match resolution {
            Resolution::Missing { estimate, why } => {
                format!("{} ({}; {why})", generation.place, estimate.short())
            }
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(format!(
        "{} of {} generated clips are missing, about ${:.2}{} in all: {list}. {}",
        missing.len(),
        outcomes.len(),
        price::ceil_cents(total),
        if unknown > 0 {
            format!(" plus {unknown} at an unknown price")
        } else {
            String::new()
        },
        if spending.ceiling_usd.is_some() {
            "Raise --max-spend, or change what the treatment asks for."
        } else {
            "Nothing was sent. To make them: --generate-clips --max-spend USD."
        }
    ))
}

/// Which generated section not made yet (a stand-in) a frame the request
/// would send shows, and why it may not be sent, if one does: the
/// frame lies inside a transition, and the side of its cut that shows is
/// such a section. A dip's falling half shows only the section before its
/// cut; a blend from the held previous picture shows both sides.
fn stand_in_in_frame(
    stills: &[(f64, PathBuf)],
    app: &crate::App,
    treatment: &Treatment,
) -> Option<(usize, String, String)> {
    use musializer_core::project::transition::Source;
    use musializer_core::treatment::format_time;
    let track = app.workspace.current()?;
    let pending: Vec<ClipGeneration> = treatment
        .clip_generations()
        .into_iter()
        .filter(|generation| !generation.attached)
        .collect();
    let cues = track.scene_switches.cues();
    for (time, _) in stills {
        let Some(active) = track
            .scene_switches
            .transition_at(*time, track.duration_seconds)
        else {
            continue;
        };
        let Some(after) = cues
            .iter()
            .position(|cue| (cue.start_seconds - active.cut_seconds).abs() < 1.0e-6)
        else {
            continue;
        };
        let sides: Vec<usize> = if *time < active.cut_seconds {
            after.checked_sub(1).into_iter().collect()
        } else if active.source == Source::Previous {
            after.checked_sub(1).into_iter().chain([after]).collect()
        } else {
            vec![after]
        };
        if let Some(missing) = pending
            .iter()
            .find(|generation| sides.contains(&generation.segment))
        {
            return Some((
                missing.segment,
                missing.place.clone(),
                format!(
                    "its frame at {} lies inside the transition at {}, which shows {}, a \
                     generated section not made yet; the frame would be a stand-in, so nothing \
                     was sent for it. Make that section first, or give this one an image file for \
                     the frame",
                    format_time(*time),
                    format_time(active.cut_seconds),
                    missing.place
                ),
            ));
        }
    }
    None
}

/// A ceiling with no typed text (the command line keeps what was typed):
/// `0.15`, or every digit the number has (`0.1234567`), never rounded. The
/// six-place form this replaced printed `0.1234567` as `0.123457` (H4d).
fn ceiling_words(ceiling: f64) -> String {
    let cents = format!("{ceiling:.2}");
    if (cents.parse::<f64>().unwrap_or(f64::NAN) - ceiling).abs() < 1.0e-9 {
        cents
    } else {
        format!("{ceiling}")
    }
}

fn estimate_without_frames(
    generation: &ClipGeneration,
    catalog: Option<&Catalog>,
    app: &crate::App,
    resolution_override: Option<(u32, u32)>,
) -> Price {
    let output = resolution_override.or_else(|| {
        app.workspace
            .current()
            .map(|track| (track.render_config.width, track.render_config.height))
    });
    let shape = catalog
        .and_then(|catalog| catalog.offer(&generation.spec.model))
        .zip(output)
        .and_then(|(offer, output)| price::shape(offer, output).ok());
    match shape {
        Some(shape) => {
            let frames = u32::from(generation.spec.first.is_some())
                + u32::from(generation.spec.last.is_some());
            price::estimate(
                catalog,
                super::now_unix(),
                &generation.spec.model,
                generation.spec.seconds,
                &shape,
                frames,
            )
        }
        None => Price::Unknown(price::UnknownPrice::NotListed),
    }
}

fn attach(
    treatment: &mut Treatment,
    entries: &mut Vec<(String, TrackClip)>,
    generation: &ClipGeneration,
    entry: TrackClip,
) -> Result<(), String> {
    treatment
        .attach_clip(&generation.key, entry.asset.clone())
        .map_err(|error| error.to_string())?;
    entries.push((generation.key.clone(), entry));
    Ok(())
}

fn resolve(treatment: &Path, name: &str) -> PathBuf {
    let declared = Path::new(name);
    if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        treatment.parent().unwrap_or(Path::new(".")).join(declared)
    }
}

/// An image file's bytes: a regular file, bounded, PNG or JPEG.
fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc_nonblock())
        .open(path)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!("larger than {MAX_IMAGE_BYTES} bytes"));
    }
    if super::client::image_mime(&bytes).is_none() {
        return Err("a frame image is a PNG or a JPEG".into());
    }
    Ok(bytes)
}

/// `O_NONBLOCK`, so naming a FIFO cannot park the run (AGENTS.md, "A check on
/// a name says nothing about the file you then open").
const fn libc_nonblock() -> i32 {
    0o4000
}

/// A PNG's size from its header.
#[must_use]
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((word(16), word(20)))
}

/// Renders stills through the export's own path at `size` (the request's),
/// restoring the track's output size after.
#[allow(clippy::too_many_arguments, reason = "as `export_stills`")]
pub fn render_frames(
    rl: &mut RaylibHandle,
    thread: &RaylibThread,
    audio: &RaylibAudio,
    music: &Option<raylib::prelude::Music<'_>>,
    app: &mut crate::App,
    analysis: &mut crate::Analysis,
    renderer: &mut SceneRenderer,
    fonts: &Faces,
    stills: &[(f64, PathBuf)],
    size: (u32, u32),
) -> Result<(), String> {
    let saved = app.workspace.current().map(|track| track.render_config);
    if let Some(track) = app.workspace.current_mut() {
        track.render_config.width = size.0;
        track.render_config.height = size.1;
    }
    let result = crate::ui::panels::export::export_stills(
        rl,
        thread,
        audio,
        music.as_ref(),
        app,
        analysis,
        renderer,
        fonts,
        stills,
    );
    if let (Some(saved), Some(track)) = (saved, app.workspace.current_mut()) {
        track.render_config = saved;
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frames_around_a_cut_are_the_last_before_it_and_the_first_at_it() {
        // 30 fps, a cut at 4.0 s: frame 119 (3.9667 s) is the last before,
        // frame 120 the first in.
        assert_eq!(frames_around(4.0, 30), (Some(119), 120));
        // A cut between frames: 4.01 s is inside frame 120.
        assert_eq!(frames_around(4.01, 30), (Some(120), 121));
        assert_eq!(frames_around(0.0, 30), (None, 0));
        let middle = frame_middle(119, 30);
        assert_eq!((middle * 30.0).floor() as u64, 119);
        assert!(middle < 4.0);
    }

    #[test]
    fn a_ceiling_is_shown_as_given_and_never_rounded() {
        assert_eq!(ceiling_words(0.15), "0.15");
        assert_eq!(ceiling_words(1.0), "1.00");
        assert_eq!(ceiling_words(0.125), "0.125");
        assert_eq!(ceiling_words(0.129_9), "0.1299");
        assert_eq!(ceiling_words(0.123_456_7), "0.1234567");
        assert_eq!(Ceiling::new(0.123_456_7, "0.1234567").shown(), "$0.1234567");
    }

    #[test]
    fn a_png_header_gives_its_size() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&1280u32.to_be_bytes());
        header.extend_from_slice(&720u32.to_be_bytes());
        assert_eq!(png_size(&header), Some((1280, 720)));
        assert_eq!(png_size(b"GIF89a"), None);
    }
}
