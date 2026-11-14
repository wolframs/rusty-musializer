//! A treatment's generated clips, in a batch run (VG H4).
//!
//! Before the treatment is applied, each section whose clip a model is to
//! make is resolved in time order: its frames are rendered through the
//! export's own still path from a draft of the treatment (the section and any
//! other unmade one standing in, the cut into the framed section plain), the
//! request is built and identified, and the clip it names is looked for — in
//! the open project's library by its provenance, then in the store. A clip
//! that is found costs nothing and needs no flag.
//!
//! **A run never spends by itself.** What is missing is refused by name with
//! its estimate and the total, and the run exits 1, unless it carries
//! `--generate-clips --max-spend USD`. Then what is missing is asked for, one
//! job at a time, while the estimates of the jobs submitted stay within the
//! ceiling; the rest are refused by name. An unknown price is refused under
//! a ceiling, since no ceiling can be kept with it.
//!
//! **A request whose earlier send lost its answer** (an intent left in the
//! store, H4b) may have been billed. It counts against the ceiling at its
//! estimate, and it is refused by name unless the run also carries
//! `--resend-unconfirmed`; then sending it again counts a second estimate.

use std::path::{Path, PathBuf};

use musializer_core::generation::catalog::Catalog;
use musializer_core::generation::price::{self, Price};
use musializer_core::generation::request::{FrameImage, GenerationRequest};
use musializer_core::project::sha256;
use musializer_core::treatment::{ClipGeneration, FrameSource, Treatment};
use musializer_runtime::font::Faces;
use raylib::prelude::{RaylibAudio, RaylibHandle, RaylibThread};

use super::job::{GenerationJob, JobSpec, Outcome, Reuse};
use super::store::Store;
use crate::clips::TrackClip;
use crate::scene_host::SceneRenderer;

/// The largest frame image file a treatment may name.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// What a run may spend.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spending {
    /// `--generate-clips` with `--max-spend USD`.
    pub ceiling_usd: Option<f64>,
    /// `--resend-unconfirmed`: a request sent before whose answer never
    /// arrived may be sent again (it may then be paid twice).
    pub resend_unconfirmed: bool,
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

/// Resolves every generated section of the treatment at `path`.
///
/// Returns `None` when it has none. Prints one `generate:` line per section.
///
/// # Errors
///
/// A section that cannot be resolved, or missing clips the run may not (or
/// could not) pay for — after every section has been looked at, so the
/// refusal lists them all with the total.
#[allow(
    clippy::too_many_arguments,
    reason = "the composition root's resources one offline frame needs, as `export_stills`"
)]
pub fn prepare<'audio>(
    path: &Path,
    spending: Spending,
    session_key: Option<&musializer_core::assist::secret::Secret>,
    resolution_override: Option<(u32, u32)>,
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

    let mut entries: Vec<(String, TrackClip)> = Vec::new();
    let mut committed = 0.0f64;
    let mut outcomes: Vec<(ClipGeneration, Resolution)> = Vec::new();
    let mut blocked: Vec<usize> = Vec::new();
    let (frames_name, frames_dir) = store.private_work("frames")?;

    for generation in treatment.clip_generations() {
        // A section whose previous one is still missing has no frame to start on.
        if generation.spec.first == Some(FrameSource::Previous)
            && blocked.contains(&(generation.segment - 1))
        {
            let estimate =
                estimate_without_frames(&generation, catalog.as_ref(), app, resolution_override);
            println!(
                "generate:        {} waits for the section before it, which is missing ({})",
                generation.place,
                estimate.short()
            );
            blocked.push(generation.segment);
            outcomes.push((
                generation,
                Resolution::Missing {
                    estimate,
                    why: "its first frame is the section before's, which is missing".into(),
                },
            ));
            continue;
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
            path,
            Some(Prepared {
                treatment: draft,
                entries: entries.clone(),
            }),
            audio,
            analysis,
            music,
            app,
            scratch,
            false,
            0.0,
        )
        .map_err(|error| format!("{}: the draft for its frames: {error}", generation.place))?;
        let (output, fps) = {
            let track = app
                .workspace
                .current()
                .ok_or("a treatment directs a track: name one in its audio block")?;
            (
                resolution_override
                    .unwrap_or((track.render_config.width, track.render_config.height)),
                track.render_config.fps,
            )
        };
        let Some(offer) = catalog
            .as_ref()
            .and_then(|catalog| catalog.offer(&generation.spec.model))
        else {
            return Err(format!(
                "{}: {} is not on the price list ({prices_line}); `--vocabulary` lists the models",
                generation.place, generation.spec.model
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
                let file = frames_dir.join(format!("{}-first.png", generation.segment));
                stills.push((frame_middle(index, fps), file.clone()));
                first = Some(file);
            }
            Some(FrameSource::File(name)) => first = Some(resolve(path, name)),
            Some(FrameSource::Next) | None => {}
        }
        match &generation.spec.last {
            Some(FrameSource::Next) => {
                let next_at = generation
                    .next_at
                    .ok_or_else(|| format!("{}.last: no section after it", generation.place))?;
                let (_, index) = frames_around(next_at, fps);
                let file = frames_dir.join(format!("{}-last.png", generation.segment));
                stills.push((frame_middle(index, fps), file.clone()));
                last = Some(file);
            }
            Some(FrameSource::File(name)) => last = Some(resolve(path, name)),
            Some(FrameSource::Previous) | None => {}
        }
        if !stills.is_empty() {
            render_frames(
                rl, thread, audio, music, app, analysis, renderer, fonts, &stills, shape.size,
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
            catalog.as_ref(),
            super::now_unix(),
            &offer.id,
            request.seconds,
            &shape,
            request.frame_count(),
        );

        // Found: in the open project, or in the store.
        let in_project = app.workspace.current().and_then(|track| {
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
            None => store.find(&digest).and_then(|(file, record)| {
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
            attach(&mut treatment, &mut entries, &generation, entry)?;
            outcomes.push((generation, Resolution::Found));
            continue;
        }

        // A job submitted before, or one another caller is sending now: it is
        // picked up rather than sent again, and it counts against the ceiling
        // at its estimate all the same — the money is this request's.
        let resumable = store.job(&digest).is_some() || store.in_flight(&digest);
        let leftover = store.leftover_intent(&digest);
        println!(
            "generate:        {} missing: {} s of {} at {} {}, {}{} — {}",
            generation.place,
            request.seconds,
            offer.id,
            shape.resolution,
            shape.aspect_ratio,
            estimate.short(),
            if resumable {
                " (submitted before, or being sent by another window or run: it is picked up, \
                 not sent again, and counts against the ceiling at its estimate)"
            } else {
                ""
            },
            request.leaves()
        );
        let earlier = leftover.as_ref().map(|intent| {
            let words = super::job::unconfirmed_words(intent);
            println!("generate:        {} {words}", generation.place);
            (intent.clone(), words)
        });
        let Some(ceiling) = spending.ceiling_usd else {
            blocked.push(generation.segment);
            outcomes.push((
                generation,
                Resolution::Missing {
                    estimate,
                    why: match &earlier {
                        Some((_, words)) => {
                            format!("the run has no --generate-clips --max-spend. {words}")
                        }
                        None => "the run has no --generate-clips --max-spend".into(),
                    },
                },
            ));
            continue;
        };
        // An earlier send that may have been billed is money this run
        // counts as spent, whether or not it sends again.
        let mut earlier_refusal = None;
        if let Some((intent, words)) = &earlier {
            match intent.estimate_usd {
                Some(usd) => committed += usd,
                None => {
                    earlier_refusal = Some(format!(
                        "{words} Its price is unknown, so no ceiling can count it"
                    ))
                }
            }
            if earlier_refusal.is_none() && !spending.resend_unconfirmed {
                earlier_refusal = Some(format!(
                    "{words} It counts against the ceiling. To send it again, which is a second \
                     request ({} more), add --resend-unconfirmed",
                    estimate.short()
                ));
            }
        }
        let cost_now = estimate.usd();
        let refusal = match cost_now {
            _ if earlier_refusal.is_some() => earlier_refusal,
            _ if ceiling <= 0.0 => Some("--max-spend 0 sends nothing".to_owned()),
            None => Some(format!(
                "{}; a run with a ceiling cannot pay an unknown price",
                match &estimate {
                    Price::Unknown(reason) => reason.reason(),
                    Price::Known(_) => String::new(),
                }
            )),
            Some(usd) if committed + usd > ceiling + 1.0e-9 => Some(format!(
                "it would pass the ceiling: ${:.2} committed this run, ${:.2} more, ${ceiling:.2} allowed",
                committed, usd
            )),
            Some(_) => None,
        };
        if let Some(why) = refusal {
            println!("generate:        {} refused: {why}", generation.place);
            blocked.push(generation.segment);
            outcomes.push((generation, Resolution::Missing { estimate, why }));
            continue;
        }
        let secret = super::credential(session_key)
            .map_err(|error| format!("{}: {error}", generation.place))?;
        let endpoint = super::client::Endpoint::from_env()?;
        let (poll, deadline, max_bytes) = JobSpec::timings_from_env();
        committed += cost_now.unwrap_or(0.0);
        let job = GenerationJob::spawn(JobSpec {
            name: super::job::clip_name(&request),
            first_png: first.map(|(bytes, _)| bytes),
            last_png: last.map(|(bytes, _)| bytes),
            estimate_usd: estimate.usd(),
            request,
            secret,
            endpoint,
            store: store.clone(),
            poll,
            deadline,
            max_bytes,
            resend: earlier.map(|(intent, _)| intent.sent_utc),
        });
        let place = generation.place.clone();
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
                    "generate:        {} {}: {} ({}x{}, {:.2} s), estimate {}, reported {}",
                    generation.place,
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
            Outcome::Failed { reason, billing } => {
                let why = match billing.sentence() {
                    Some(text) => format!("{}. {text}", reason.trim_end_matches('.')),
                    None => reason,
                };
                println!("generate:        {} failed: {why}", generation.place);
                blocked.push(generation.segment);
                outcomes.push((generation, Resolution::Missing { estimate, why }));
            }
            Outcome::Unconfirmed(intent) => {
                let why = format!(
                    "{} Nothing was sent now; add --resend-unconfirmed to send it again",
                    super::job::unconfirmed_words(&intent)
                );
                println!("generate:        {} refused: {why}", generation.place);
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
            spent.max(0.0) + 0.0
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
    fn a_png_header_gives_its_size() {
        let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        header.extend_from_slice(&1280u32.to_be_bytes());
        header.extend_from_slice(&720u32.to_be_bytes());
        assert_eq!(png_size(&header), Some((1280, 720)));
        assert_eq!(png_size(b"GIF89a"), None);
    }
}
