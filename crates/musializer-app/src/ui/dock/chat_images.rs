//! Pictures in the agent chat: finding them, drawing them, looking closer.
//!
//! **Finding.** `ChatImages::scan` runs once a frame from `ChatTab::tick`,
//! visible or not. An entry is read once, when `Transcript::entry_settled` says
//! its words are final, so streamed text cannot give half a path and no text
//! is read twice. `core::agent::image_paths` says what it spells, and a thread
//! of the chat's own looks at the disk: **nothing here waits on a filesystem on
//! the UI thread**, because a dead network mount would freeze the window for a
//! chat nobody is looking at. That thread answers in the order it was asked,
//! holds what has shown, and each answer appends one `Entry::Image` per picture.
//! A later mention only shows again when the file's mtime moved.
//!
//! **Drawing needs the entry and the working directory, nothing else**, which
//! is what lets a saved chat reopen. A transcript put into a chat after its
//! first tick needs `resume`, or its old text reads as new mentions.
//!
//! **Decoding** happens on a second thread, one per chat, started by the first
//! picture, so a slow disk under one file cannot hold up the search for the
//! next. Each picture is shrunk to the size it is drawn at, and only the ones
//! on screen are decoded, so a long chat holds `MOST_THUMBS` textures however
//! many pictures it mentions. The large view takes a second decode, at the
//! window's size, and shows the thumbnail until that arrives. Every job carries
//! a generation, and only the current one is accepted. Pixels waiting for the
//! UI thread are bounded in count and bytes; the worker waits for room, and a
//! chat that is not on screen throws them away, since it has no frame to
//! upload them in. Reading and budgets are `runtime::decode::with_picture`'s.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use egui::emath::GuiRounding;
use egui::{Color32, ColorImage, RichText, Stroke, StrokeKind, TextureHandle, TextureOptions};
use musializer_core::agent::{image_paths, Entry, Transcript};
use musializer_runtime::decode::{with_picture, PictureError, PICTURE_LARGEST_FILE};

use super::{Body, Dock};
use crate::ui::theme::color32;

/// A tool listing a folder of stills must not fill the chat. A picture past
/// this stays unseen, so naming it again shows it.
const MOST_FROM_ONE_ENTRY: usize = 6;
/// Bounds the disk checks one entry can cost.
const MOST_CANDIDATES: usize = 64;
const MOST_THUMBS: usize = 24;
/// The same bound in bytes: a thumbnail at 4× scale is sixteen times one at 1×.
const MOST_THUMB_BYTES: usize = 128 * 1024 * 1024;
/// Jobs waiting for the decoder. Past this a picture stays `Loading…` and asks
/// again next frame, if it is still on screen.
const MOST_JOBS: usize = 16;
/// Finished pictures waiting for the UI thread, by count and by bytes. The
/// worker starts nothing while either is full, so one result can overrun the
/// bytes, and no more than one.
const MOST_RESULTS: usize = 4;
const MOST_RESULT_BYTES: usize = 64 * 1024 * 1024;
/// A queued picture nothing has drawn for this many frames has scrolled away.
const STALE_FRAMES: u64 = 30;
/// What closing a chat waits for its worker, which has a running FFmpeg to
/// kill. A worker stuck in a dead mount is left behind, not waited for.
const WORKER_EXIT: Duration = Duration::from_millis(300);
/// A portrait still at the column's width would be taller than the column.
const TALLEST_THUMB: f32 = 360.0;
/// What a picture of unknown shape holds: most stills are 16:9.
const PLACEHOLDER_ASPECT: f32 = 9.0 / 16.0;
const CORNER: f32 = 6.0;
const CAPTION: f32 = 11.0;
/// Around the large picture: the window's margin, then its caption's row.
const LARGE_MARGIN: f32 = 24.0;
const LARGE_CAPTION: f32 = 36.0;

/// Why a picture is not there. Drawn in its place, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Missing,
    TooLarge,
    Undecodable,
}

impl Failure {
    fn report(self) -> &'static str {
        match self {
            Failure::Missing => "missing",
            Failure::TooLarge => "too-large",
            Failure::Undecodable => "undecodable",
        }
    }

    fn says(self, name: &str) -> String {
        match self {
            Failure::Missing => format!("{name} is missing"),
            Failure::TooLarge => format!("{name} is too large to show"),
            Failure::Undecodable => format!("{name} could not be read as a picture"),
        }
    }
}

struct Job {
    path: PathBuf,
    large: bool,
    /// Which request this is; only the newest for a picture counts.
    generation: u64,
    /// Up when nobody wants the answer any more.
    cancel: Arc<AtomicBool>,
    /// The most pixels it will be drawn at.
    bounds: [usize; 2],
}

struct Decoded {
    image: ColorImage,
    natural: [usize; 2],
}

struct Done {
    path: PathBuf,
    large: bool,
    generation: u64,
    result: Result<Decoded, Failure>,
}

impl Done {
    fn bytes(&self) -> usize {
        self.result
            .as_ref()
            .map_or(0, |decoded| decoded.image.pixels.len() * 4)
    }
}

/// `None` when cancelled: nobody is waiting for that answer.
fn decode(job: &Job) -> Option<Result<Decoded, Failure>> {
    let bounds = job.bounds;
    let decoded = with_picture(&job.path, &job.cancel, |pixels, natural| Decoded {
        image: shrink(pixels, natural, shrunk(natural, bounds)),
        natural,
    });
    match decoded {
        Ok(decoded) => Some(Ok(decoded)),
        Err(PictureError::Missing) => Some(Err(Failure::Missing)),
        Err(PictureError::TooLarge) => Some(Err(Failure::TooLarge)),
        Err(PictureError::Undecodable) => Some(Err(Failure::Undecodable)),
        Err(PictureError::Cancelled) => None,
    }
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    results: VecDeque<Done>,
    result_bytes: usize,
    /// What the worker is on, so closing can stop it.
    running: Option<Arc<AtomicBool>>,
    closed: bool,
    /// The worker has returned.
    finished: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn decode_jobs(shared: &Shared) {
    loop {
        let job = {
            let mut queue = shared.lock();
            loop {
                if queue.closed {
                    queue.finished = true;
                    shared.changed.notify_all();
                    return;
                }
                let room =
                    queue.results.len() < MOST_RESULTS && queue.result_bytes < MOST_RESULT_BYTES;
                // Newest first: that is what the user is looking at.
                let next = if room { queue.jobs.pop_back() } else { None };
                if let Some(job) = next {
                    queue.running = Some(job.cancel.clone());
                    break job;
                }
                queue = shared
                    .changed
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let result = decode(&job);
        let mut queue = shared.lock();
        queue.running = None;
        if let Some(result) = result.filter(|_| !job.cancel.load(Ordering::Relaxed)) {
            let done = Done {
                path: job.path,
                large: job.large,
                generation: job.generation,
                result,
            };
            queue.result_bytes += done.bytes();
            queue.results.push_back(done);
        }
    }
}

/// The chat's end of the decoding thread.
struct Decoder {
    shared: Arc<Shared>,
}

impl Decoder {
    fn start() -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("chat-images".to_owned())
            .spawn(move || decode_jobs(&worker))
            .ok()?;
        Some(Self { shared })
    }

    /// Queues `job`. A large view always fits: there is one, and it replaced
    /// the last. Hands the job back when the queue is full.
    fn submit(&self, job: Job) -> Result<(), Job> {
        let mut queue = self.shared.lock();
        if !job.large && queue.jobs.len() >= MOST_JOBS {
            return Err(job);
        }
        queue.jobs.push_back(job);
        self.shared.changed.notify_all();
        Ok(())
    }

    fn withdraw(&self, generation: u64) {
        self.shared
            .lock()
            .jobs
            .retain(|job| job.generation != generation);
    }

    /// Everything queued goes, and what is running stops.
    fn stop_all(&self) {
        let mut queue = self.shared.lock();
        for job in queue.jobs.drain(..) {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(running) = &queue.running {
            running.store(true, Ordering::Relaxed);
        }
    }

    /// The finished pictures, which makes room for more.
    fn take_results(&self) -> VecDeque<Done> {
        let mut queue = self.shared.lock();
        queue.result_bytes = 0;
        let results = std::mem::take(&mut queue.results);
        self.shared.changed.notify_all();
        results
    }
}

impl Drop for Decoder {
    /// Waits, a little, so a running FFmpeg is dead before the application is.
    fn drop(&mut self) {
        self.stop_all();
        let mut queue = self.shared.lock();
        queue.closed = true;
        self.shared.changed.notify_all();
        let (queue, _) = self
            .shared
            .changed
            .wait_timeout_while(queue, WORKER_EXIT, |queue| !queue.finished)
            .unwrap_or_else(PoisonError::into_inner);
        drop(queue);
    }
}

/// What the disk thread is asked, in order.
enum Ask {
    /// What a reopened chat already shows, so it does not show it twice.
    Seed {
        directory: Option<PathBuf>,
        paths: Vec<String>,
    },
    /// What one settled entry named.
    Look {
        directory: Option<PathBuf>,
        candidates: Vec<String>,
    },
    /// Stands in for a dead mount in a test.
    #[cfg(test)]
    Wedge(Receiver<()>),
}

/// The pictures to append for one `Look`, and how many there were to show.
struct Answer {
    paths: Vec<PathBuf>,
    total: usize,
}

/// A rewritten picture has to be read again, where it first showed too: an
/// entry is a path, not a moment. Hence the caller forgets its texture.
fn look(
    seen: &mut HashMap<PathBuf, SystemTime>,
    directory: Option<&Path>,
    candidates: &[String],
) -> Answer {
    let mut found: Vec<(PathBuf, PathBuf, SystemTime)> = Vec::new();
    for candidate in candidates.iter().take(MOST_CANDIDATES) {
        let path = resolve(directory, candidate);
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > PICTURE_LARGEST_FILE {
            continue;
        }
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.get(&real) != Some(&modified) && found.iter().all(|(_, r, _)| *r != real) {
            found.push((path, real, modified));
        }
    }
    let total = found.len();
    let paths = found
        .into_iter()
        .take(MOST_FROM_ONE_ENTRY)
        .map(|(path, real, modified)| {
            seen.insert(real, modified);
            path
        })
        .collect();
    Answer { paths, total }
}

/// Holds what has shown, by real path and mtime. Ends with the chat's sender,
/// and a thread stuck in a dead mount ends when the mount answers.
fn answer_lookups(asks: Receiver<Ask>, answers: Sender<Answer>) {
    let mut seen = HashMap::new();
    for ask in asks {
        match ask {
            Ask::Seed { directory, paths } => {
                for path in paths {
                    let path = resolve(directory.as_deref(), &path);
                    if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                        let real = std::fs::canonicalize(&path).unwrap_or(path);
                        seen.insert(real, modified);
                    }
                }
            }
            Ask::Look {
                directory,
                candidates,
            } => {
                let answer = look(&mut seen, directory.as_deref(), &candidates);
                if answers.send(answer).is_err() {
                    return;
                }
            }
            #[cfg(test)]
            Ask::Wedge(release) => {
                let _ = release.recv();
            }
        }
    }
}

/// The size that fits `bounds` and keeps the shape, never larger than `natural`.
fn shrunk(natural: [usize; 2], bounds: [usize; 2]) -> [usize; 2] {
    let [width, height] = natural.map(|side| side as f32);
    let scale = (bounds[0] as f32 / width)
        .min(bounds[1] as f32 / height)
        .min(1.0);
    natural.map(|side| ((side as f32 * scale).round() as usize).clamp(1, side))
}

/// A box filter over premultiplied colour, so a transparent pixel's hidden
/// colour cannot bleed into an edge. The texture is premultiplied, as egui's
/// are. An opaque colour comes through exact, which the gate's capture pins.
fn shrink(pixels: &[u8], from: [usize; 2], to: [usize; 2]) -> ColorImage {
    let span = |index: usize, whole: usize, parts: usize| {
        let start = index * whole / parts;
        (start, ((index + 1) * whole / parts).max(start + 1))
    };
    let [source_width, source_height] = from;
    let [width, height] = to;
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        let (top, bottom) = span(y, source_height, height);
        for x in 0..width {
            let (left, right) = span(x, source_width, width);
            let mut sum = [0_u64; 4];
            for row in top..bottom {
                let start = row * source_width;
                for pixel in pixels[(start + left) * 4..(start + right) * 4].chunks_exact(4) {
                    let alpha = u64::from(pixel[3]);
                    sum[0] += u64::from(pixel[0]) * alpha;
                    sum[1] += u64::from(pixel[1]) * alpha;
                    sum[2] += u64::from(pixel[2]) * alpha;
                    sum[3] += alpha;
                }
            }
            let count = ((bottom - top) * (right - left)) as u64;
            let scale = count * 255;
            let colour = |value: u64| ((value + scale / 2) / scale) as u8;
            out.push(Color32::from_rgba_premultiplied(
                colour(sum[0]),
                colour(sum[1]),
                colour(sum[2]),
                ((sum[3] + count / 2) / count) as u8,
            ));
        }
    }
    ColorImage::new(to, out)
}

/// `path` as a file: `~/` is the home folder, and anything else not absolute
/// is under the chat's working directory. A `..` stays: through a symlink only
/// the filesystem knows what it means. Touches no disk, so `show` can call it.
fn resolve(directory: Option<&Path>, path: &str) -> PathBuf {
    let home = path
        .strip_prefix("~/")
        .and_then(|rest| Some(Path::new(&std::env::var_os("HOME")?).join(rest)));
    let path = home.unwrap_or_else(|| PathBuf::from(path));
    let whole = if path.is_absolute() {
        path
    } else {
        directory
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
            .join(path)
    };
    whole.components().collect()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

enum Thumb {
    Decoding {
        generation: u64,
        cancel: Arc<AtomicBool>,
        /// The last frame it was on screen.
        wanted: u64,
    },
    Ready {
        texture: TextureHandle,
        natural: [usize; 2],
        last_drawn: u64,
    },
    Failed(Failure),
}

/// The picture over the window.
struct Large {
    path: PathBuf,
    name: String,
    natural: [usize; 2],
    thumbnail: TextureHandle,
    full: Option<TextureHandle>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    bounds: [usize; 2],
    /// The full-size decode failed; the thumbnail stays and says so.
    failed: Option<Failure>,
    /// `true` until its first frame: the composer keeps egui's focus until then.
    opening: bool,
    /// The composer had the keyboard. Closing gives it back then and only
    /// then: a chat nobody was typing in must not take Space from the
    /// transport.
    return_focus: bool,
}

/// What the large picture did this frame.
#[derive(Default)]
pub struct LargeFrame {
    /// Open now, or closed by this frame's Escape, which must not also reach
    /// the shell.
    pub takes_keyboard: bool,
    /// Closed, and the composer should have the keyboard back.
    pub refocus: bool,
}

#[derive(Default)]
pub struct ChatImages {
    directory: Option<PathBuf>,
    /// What `scan` has read, by entry index; append-only, as the entries are.
    read: Vec<bool>,
    /// Every entry below this has been read.
    first_unread: usize,
    /// The first tick has run: whatever was already there is not news.
    started: bool,
    /// The disk thread's two ends, once something has been asked.
    asks: Option<Sender<Ask>>,
    answers: Option<Receiver<Answer>>,
    /// Looks sent and not yet answered.
    unanswered: usize,

    decoder: Option<Decoder>,
    next_generation: u64,
    thumbs: HashMap<PathBuf, Thumb>,
    /// Pictures' natural sizes, outliving their textures, so one that was
    /// evicted comes back into exactly the room it left.
    sizes: HashMap<PathBuf, [usize; 2]>,
    large: Option<Large>,
    frame: u64,
    /// The composer's focus as this frame started; a press on a thumbnail
    /// takes it away on the frame it lands.
    composer_focused: bool,
    /// `--ui-probe agent-image-hold=`: finished decodes wait here.
    held: bool,
    /// Ticks so far, and the last one a frame drew this chat. A chat ticks
    /// whether or not it is on screen.
    ticks: u64,
    shown_at: u64,
    /// What the frame drew, for `dock:`.
    drawn: Vec<String>,
}

impl ChatImages {
    /// Made absolute here: a track opened by a relative path gives a relative
    /// directory, and a saved entry must not depend on where this was started.
    pub fn set_directory(&mut self, directory: Option<PathBuf>) {
        self.directory = directory.map(|dir| std::path::absolute(&dir).unwrap_or(dir));
    }

    fn ask(&mut self, ask: Ask) {
        if self.asks.is_none() {
            let (asks, incoming) = mpsc::channel();
            let (answering, answers) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("chat-image-lookups".to_owned())
                .spawn(move || answer_lookups(incoming, answering));
            if worker.is_err() {
                return;
            }
            self.asks = Some(asks);
            self.answers = Some(answers);
        }
        // Words, not pixels, and no more of them than the transcript holds,
        // so a stuck disk cannot grow this past the chat itself.
        if let Some(asks) = &self.asks {
            let _ = asks.send(ask);
        }
    }

    /// Takes everything in `entries` as already read, and what is already
    /// shown as seen. The first tick does this by itself. The thread takes
    /// asks in order, so any later mention finds this done.
    pub fn resume(&mut self, entries: &[Entry]) {
        self.started = true;
        self.read = vec![true; entries.len()];
        self.first_unread = entries.len();
        let paths: Vec<String> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Image { path } => Some(path.clone()),
                _ => None,
            })
            .collect();
        if !paths.is_empty() {
            self.ask(Ask::Seed {
                directory: self.directory.clone(),
                paths,
            });
        }
    }

    /// Reads the entries that have settled since the last frame, appends
    /// what the disk thread has answered, and keeps a hidden chat's decoding
    /// from piling up. Returns whether the transcript grew.
    pub fn scan(&mut self, transcript: &mut Transcript) -> bool {
        self.ticks += 1;
        if self.ticks > self.shown_at + 1 {
            self.stand_down();
        }
        if !self.started {
            self.resume(&transcript.entries);
            return false;
        }
        let grew = self.take_answers(transcript);
        let end = transcript.entries.len();
        if self.first_unread == end {
            return grew;
        }
        self.read.resize(end, false);
        for index in self.first_unread..end {
            if self.read[index] || !transcript.entry_settled(index) {
                continue;
            }
            self.read[index] = true;
            let candidates = match &transcript.entries[index] {
                Entry::Assistant { text } => image_paths(text),
                Entry::Tool { detail, output, .. } => {
                    let mut candidates = image_paths(detail);
                    candidates.extend(image_paths(output));
                    let mut named = transcript.take_tool_image_paths(index);
                    named.append(&mut candidates);
                    named
                }
                _ => Vec::new(),
            };
            if !candidates.is_empty() {
                self.unanswered += 1;
                self.ask(Ask::Look {
                    directory: self.directory.clone(),
                    candidates,
                });
            }
        }
        while self.read.get(self.first_unread) == Some(&true) {
            self.first_unread += 1;
        }
        grew
    }

    fn take_answers(&mut self, transcript: &mut Transcript) -> bool {
        let mut grew = false;
        let Some(answers) = self.answers.take() else {
            return false;
        };
        while let Ok(answer) = answers.try_recv() {
            self.unanswered = self.unanswered.saturating_sub(1);
            let shown = answer.paths.len();
            for path in answer.paths {
                self.forget(&path);
                transcript.entries.push(Entry::Image {
                    path: path.display().to_string(),
                });
                grew = true;
            }
            if answer.total > shown {
                transcript.entries.push(Entry::Notice {
                    text: format!("Showing {shown} of {} pictures named here", answer.total),
                    error: false,
                });
            }
        }
        self.answers = Some(answers);
        grew
    }

    /// Drops what is known of a picture, because its file changed: a decode
    /// still running is stale, and an open large view reads the new one.
    fn forget(&mut self, path: &Path) {
        if let Some(Thumb::Decoding {
            generation, cancel, ..
        }) = self.thumbs.get(path)
        {
            cancel.store(true, Ordering::Relaxed);
            if let Some(decoder) = &self.decoder {
                decoder.withdraw(*generation);
            }
        }
        self.thumbs.remove(path);
        self.sizes.remove(path);
        let bounds = match self.large.as_ref().filter(|open| open.path == path) {
            Some(open) => open.bounds,
            None => return,
        };
        self.request_large(path.to_path_buf(), bounds);
    }

    /// This chat is not on screen, so it has no frame to upload pixels in and
    /// nothing to decode for. Whatever comes back into view asks again.
    fn stand_down(&mut self) {
        let Some(decoder) = &self.decoder else {
            return;
        };
        decoder.take_results();
        if self
            .thumbs
            .values()
            .any(|thumb| matches!(thumb, Thumb::Decoding { .. }))
        {
            decoder.stop_all();
            self.thumbs
                .retain(|_, thumb| !matches!(thumb, Thumb::Decoding { .. }));
        }
    }

    fn generation(&mut self) -> (u64, Arc<AtomicBool>) {
        self.next_generation += 1;
        (self.next_generation, Arc::new(AtomicBool::new(false)))
    }

    /// `None` when there is no worker to give it to.
    fn decoder(&mut self) -> Option<&Decoder> {
        if self.decoder.is_none() {
            self.decoder = Decoder::start();
        }
        self.decoder.as_ref()
    }

    /// Asks for a thumbnail. A full queue leaves it unasked, for the next
    /// frame; no worker at all is a failure to say out loud.
    fn request_thumb(&mut self, path: PathBuf, bounds: [usize; 2]) {
        let (generation, cancel) = self.generation();
        let job = Job {
            path: path.clone(),
            large: false,
            generation,
            cancel: cancel.clone(),
            bounds,
        };
        let Some(decoder) = self.decoder() else {
            self.thumbs
                .insert(path, Thumb::Failed(Failure::Undecodable));
            return;
        };
        if decoder.submit(job).is_ok() {
            let wanted = self.frame;
            self.thumbs.insert(
                path,
                Thumb::Decoding {
                    generation,
                    cancel,
                    wanted,
                },
            );
        }
    }

    /// Starts the large view's decode, in place of any before it.
    fn request_large(&mut self, path: PathBuf, bounds: [usize; 2]) {
        let (generation, cancel) = self.generation();
        let job = Job {
            path,
            large: true,
            generation,
            cancel: cancel.clone(),
            bounds,
        };
        self.stop_large_decode();
        let sent = self.decoder().is_some_and(|d| d.submit(job).is_ok());
        if let Some(open) = self.large.as_mut() {
            open.generation = generation;
            open.cancel = cancel;
            open.failed = (!sent).then_some(Failure::Undecodable);
        }
    }

    fn stop_large_decode(&self) {
        if let Some(open) = &self.large {
            open.cancel.store(true, Ordering::Relaxed);
            if let Some(decoder) = &self.decoder {
                decoder.withdraw(open.generation);
            }
        }
    }

    pub fn begin_frame(&mut self, ctx: &egui::Context, composer_focused: bool) {
        self.frame += 1;
        self.shown_at = self.ticks;
        self.composer_focused = composer_focused;
        self.drawn.clear();
        self.evict();
        self.drop_scrolled_away();
        if self.held {
            return;
        }
        let results = self
            .decoder
            .as_ref()
            .map(Decoder::take_results)
            .unwrap_or_default();
        for done in results {
            self.finish(ctx, done);
        }
    }

    /// A decode still waiting for a picture that has left the screen is work
    /// for nothing, and holds a queue place something on screen could use.
    fn drop_scrolled_away(&mut self) {
        let frame = self.frame;
        let decoder = self.decoder.as_ref();
        self.thumbs.retain(|_, thumb| {
            let Thumb::Decoding {
                generation,
                cancel,
                wanted,
            } = thumb
            else {
                return true;
            };
            if *wanted + STALE_FRAMES >= frame {
                return true;
            }
            cancel.store(true, Ordering::Relaxed);
            if let Some(decoder) = decoder {
                decoder.withdraw(*generation);
            }
            false
        });
    }

    /// Drops the thumbnails drawn longest ago, past the bounds. One drawn on
    /// the last frame stays whatever the count, which is safe: a texture holds
    /// one texel per pixel it is drawn at, so what is on screen at once cannot
    /// hold much more than a column of window height, plus the two cut by its
    /// edges, or twice that for thumbnails decoded before the column narrowed.
    /// Evicting one of those would only bring it straight back.
    fn evict(&mut self) {
        let mut ready: Vec<(u64, usize, PathBuf)> = self
            .thumbs
            .iter()
            .filter_map(|(path, thumb)| match thumb {
                Thumb::Ready {
                    texture,
                    last_drawn,
                    ..
                } => Some((*last_drawn, texture.byte_size(), path.clone())),
                _ => None,
            })
            .collect();
        let mut bytes: usize = ready.iter().map(|(_, size, _)| size).sum();
        if ready.len() <= MOST_THUMBS && bytes <= MOST_THUMB_BYTES {
            return;
        }
        ready.sort_by_key(|(last_drawn, ..)| *last_drawn);
        let mut count = ready.len();
        for (last_drawn, size, path) in ready {
            if (count <= MOST_THUMBS && bytes <= MOST_THUMB_BYTES) || last_drawn + 1 >= self.frame {
                break;
            }
            self.thumbs.remove(&path);
            count -= 1;
            bytes -= size;
        }
    }

    fn finish(&mut self, ctx: &egui::Context, done: Done) {
        let Done {
            path,
            large,
            generation,
            result,
        } = done;
        let name = format!("chat-image:{}", path.display());
        if large {
            if let Some(open) = self
                .large
                .as_mut()
                .filter(|open| open.path == path && open.generation == generation)
            {
                match result {
                    Ok(decoded) => {
                        open.natural = decoded.natural;
                        open.full =
                            Some(ctx.load_texture(name, decoded.image, TextureOptions::LINEAR));
                    }
                    Err(failure) => open.failed = Some(failure),
                }
            }
            return;
        }
        // Forgotten or asked again since: the file changed, a newer job runs.
        let current = matches!(
            self.thumbs.get(&path),
            Some(Thumb::Decoding { generation: asked, .. }) if *asked == generation
        );
        if !current {
            return;
        }
        let thumb = match result {
            Ok(decoded) => {
                self.sizes.insert(path.clone(), decoded.natural);
                Thumb::Ready {
                    texture: ctx.load_texture(name, decoded.image, TextureOptions::LINEAR),
                    natural: decoded.natural,
                    last_drawn: self.frame,
                }
            }
            Err(failure) => Thumb::Failed(failure),
        };
        self.thumbs.insert(path, thumb);
    }

    /// Holds finished decodes back, so a capture can see what a slow one looks
    /// like (`--ui-probe agent-image-hold=`).
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// One `Entry::Image`: the picture over a small caption, a placeholder of
    /// the same size while it decodes, or what went wrong, by name.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        layout: &mut Vec<(&'static str, egui::Rect)>,
    ) {
        let resolved = resolve(self.directory.as_deref(), path);
        let name = file_name(&resolved);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if let Some(Thumb::Failed(failure)) = self.thumbs.get(&resolved) {
                let failure = *failure;
                let rect = failed_card(ui, &name, failure, &resolved);
                self.drawn.push(format!("{name}:{}", failure.report()));
                layout.push(("image", rect));
                return;
            }
            self.picture(ui, resolved, &name, layout);
        });
    }

    fn picture(
        &mut self,
        ui: &mut egui::Ui,
        resolved: PathBuf,
        name: &str,
        layout: &mut Vec<(&'static str, egui::Rect)>,
    ) {
        let palette = crate::ui::theme::palette();
        let weak = ui.visuals().weak_text_color();
        let scale = ui.ctx().pixels_per_point();
        let width = ui.available_width();
        let ready = match self.thumbs.get(&resolved) {
            Some(Thumb::Ready {
                texture, natural, ..
            }) => Some((texture.clone(), *natural)),
            _ => None,
        };
        let natural = ready
            .as_ref()
            .map(|(_, natural)| *natural)
            .or_else(|| self.sizes.get(&resolved).copied());
        let size = natural.map_or_else(
            || {
                let height = (width * PLACEHOLDER_ASPECT).min(TALLEST_THUMB);
                egui::vec2(width, height)
            },
            |natural| thumbnail_size(natural, width, scale),
        );
        let sense = if ready.is_some() {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let rect = rect.round_to_pixels(ui.pixels_per_point());
        let visible = ui.is_rect_visible(rect);
        let trough = color32::of(palette.ui_lane_trough);
        ui.painter().rect_filled(rect, CORNER, trough);
        let mut edge = color32::of(palette.ui_rule);
        match &ready {
            Some((texture, natural)) => {
                if response.hovered() {
                    edge = color32::ui_ink();
                }
                if visible {
                    egui::Image::new((texture.id(), rect.size()))
                        .corner_radius(CORNER)
                        .paint_at(ui, rect);
                    if let Some(Thumb::Ready { last_drawn, .. }) = self.thumbs.get_mut(&resolved) {
                        *last_drawn = self.frame;
                    }
                }
                let [w, h] = natural;
                let [tw, th] = texture.size();
                self.drawn.push(format!(
                    "{name}:{}:{w}x{h}:{tw}x{th}",
                    if visible { "shown" } else { "off-screen" }
                ));
            }
            None => {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Loading\u{2026}",
                    egui::FontId::proportional(12.0),
                    weak,
                );
                if visible {
                    match self.thumbs.get_mut(&resolved) {
                        Some(Thumb::Decoding { wanted, .. }) => *wanted = self.frame,
                        Some(_) => {}
                        None => {
                            let bounds = [
                                (width.max(1.0) * scale).ceil() as usize,
                                (TALLEST_THUMB * scale).ceil() as usize,
                            ];
                            self.request_thumb(resolved.clone(), bounds);
                        }
                    }
                }
                self.drawn.push(format!(
                    "{name}:{}",
                    if visible { "decoding" } else { "off-screen" }
                ));
            }
        }
        ui.painter()
            .rect_stroke(rect, CORNER, Stroke::new(1.0, edge), StrokeKind::Inside);
        layout.push(("image", rect));
        let caption =
            ui.add(egui::Label::new(RichText::new(name).size(CAPTION).color(weak)).truncate());
        let tip = resolved.display().to_string();
        layout.push(("image-caption", caption.rect));
        caption.on_hover_text(tip.as_str());
        if let Some((texture, natural)) = ready {
            let response = response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Open large\n{tip}"));
            if response.clicked() {
                self.open(ui.ctx(), resolved, name.to_owned(), texture, natural);
            }
        }
    }

    fn open(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        name: String,
        thumbnail: TextureHandle,
        natural: [usize; 2],
    ) {
        let window = ctx.content_rect().size() * ctx.pixels_per_point();
        let bounds = [window.x.ceil() as usize, window.y.ceil() as usize];
        self.large = Some(Large {
            path: path.clone(),
            name,
            natural,
            thumbnail,
            full: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            bounds,
            failed: None,
            opening: true,
            return_focus: self.composer_focused,
        });
        self.request_large(path, bounds);
    }

    /// Whether the picture is open over the window.
    #[must_use]
    pub fn large_open(&self) -> bool {
        self.large.is_some()
    }

    pub fn close_large(&mut self) {
        self.stop_large_decode();
        self.large = None;
    }

    /// The picture over the window, fit to it: Escape or a press outside
    /// closes it. A modal layer, so nothing under it takes a press, and the
    /// shell's own widgets are told by `Dock::picture_open`.
    pub fn show_large(
        &mut self,
        ctx: &egui::Context,
        id: egui::Id,
        layout: &mut Vec<(&'static str, egui::Rect)>,
    ) -> LargeFrame {
        let Some(open) = self.large.as_mut() else {
            return LargeFrame::default();
        };
        if std::mem::take(&mut open.opening) {
            // Typing behind the picture would land in a draft nobody can see.
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
        let palette = crate::ui::theme::palette();
        let window = ctx.content_rect();
        let texture = open.full.as_ref().unwrap_or(&open.thumbnail);
        let size = fitted(
            open.natural,
            egui::vec2(
                window.width() - LARGE_MARGIN * 2.0,
                window.height() - LARGE_MARGIN * 2.0 - LARGE_CAPTION,
            ),
        );
        let scrim = crate::ui::theme::color::scrim();
        let [width, height] = open.natural;
        let caption = match open.failed {
            Some(failure) => failure.says(&open.name),
            None => format!(
                "{} \u{00b7} {width} \u{00d7} {height} \u{00b7} Esc closes",
                open.name
            ),
        };
        let mut picture = egui::Rect::NOTHING;
        let modal_id = id.with("chat_image_large");
        let modal = egui::Modal::new(modal_id)
            .area(egui::Modal::default_area(modal_id).fade_in(false))
            .backdrop_color(Color32::from_rgba_unmultiplied(
                scrim.r, scrim.g, scrim.b, scrim.a,
            ))
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::click());
                    let rect = rect.round_to_pixels(ui.pixels_per_point());
                    egui::Image::new((texture.id(), rect.size()))
                        .corner_radius(CORNER)
                        .paint_at(ui, rect);
                    picture = rect;
                    let ink = color32::ui_ink();
                    let words =
                        ui.painter()
                            .layout_no_wrap(caption, egui::FontId::proportional(12.0), ink);
                    let padding = egui::vec2(8.0, 3.0);
                    let (chip, _) =
                        ui.allocate_exact_size(words.size() + padding * 2.0, egui::Sense::hover());
                    ui.painter()
                        .rect_filled(chip, 4.0, color32::of(palette.ui_raised));
                    ui.painter().galley(chip.min + padding, words, ink);
                });
            });
        layout.push(("viewer", picture));
        layout.push((
            "viewer-outside",
            egui::Rect::from_min_size(window.min, egui::vec2(8.0, 8.0)),
        ));
        let closed = modal.should_close();
        let refocus = closed && open.return_focus;
        if closed {
            self.close_large();
        }
        LargeFrame {
            takes_keyboard: true,
            refocus,
        }
    }

    /// `images=[…] large=…` for `dock:`: what each picture drew this frame,
    /// so a report tells a picture from its placeholder.
    #[must_use]
    pub fn describe(&self, transcript: &Transcript) -> String {
        let entries = transcript
            .entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Image { .. }))
            .count();
        let large = self.large.as_ref().map_or_else(
            || "closed".to_owned(),
            |open| {
                let [width, height] = open.full.as_ref().unwrap_or(&open.thumbnail).size();
                format!(
                    "{}:{}:{width}x{height}",
                    open.name,
                    if open.full.is_some() {
                        "full"
                    } else {
                        "thumbnail"
                    }
                )
            },
        );
        format!(
            "image-entries={entries} images=[{}] large={large}",
            self.drawn.join(";")
        )
    }
}

/// Fits `bounds` at the window's own scale, keeping the shape.
fn fitted(natural: [usize; 2], bounds: egui::Vec2) -> egui::Vec2 {
    let [width, height] = natural.map(|side| side as f32);
    let scale = (bounds.x / width).min(bounds.y / height);
    egui::vec2((width * scale).max(1.0), (height * scale).max(1.0))
}

/// A thumbnail never magnifies: one physical pixel per picture pixel at most.
fn thumbnail_size(natural: [usize; 2], width: f32, scale: f32) -> egui::Vec2 {
    let [w, h] = natural.map(|side| side as f32 / scale);
    fitted(natural, egui::vec2(width.min(w), TALLEST_THUMB.min(h)))
}

/// What stands where a picture could not: its name and what went wrong.
fn failed_card(ui: &mut egui::Ui, name: &str, failure: Failure, path: &Path) -> egui::Rect {
    let palette = crate::ui::theme::palette();
    egui::Frame::new()
        .fill(color32::of(palette.ui_lane_trough))
        .stroke(Stroke::new(1.0, color32::of(palette.ui_rule)))
        .corner_radius(CORNER)
        .inner_margin(egui::Margin::symmetric(9, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(failure.says(name))
                        .size(12.0)
                        .color(color32::ui_danger()),
                )
                .wrap(),
            );
            let folder = path.parent().unwrap_or(path).display().to_string();
            ui.add(
                egui::Label::new(
                    RichText::new(folder)
                        .size(CAPTION)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate(),
            );
        })
        .response
        .rect
}

impl Dock {
    /// Whether a chat's picture is open over the window. The shell's widgets
    /// stand down for it, as they do for About.
    #[must_use]
    pub fn picture_open(&self) -> bool {
        self.tabs
            .iter()
            .any(|tab| matches!(&tab.body, Body::Chat(chat) if chat.images.large_open()))
    }

    /// Closes it: the chat that drew it has left the screen.
    pub fn close_picture(&mut self) {
        for tab in &mut self.tabs {
            if let Body::Chat(chat) = &mut tab.body {
                chat.images.close_large();
            }
        }
    }

    /// Adds a picture to the shown chat as a saved chat holds one
    /// (`--ui-probe agent-image=`).
    pub fn probe_image(&mut self, path: &str) {
        let index = self.current_chat().unwrap_or(usize::MAX);
        if let Some(Body::Chat(chat)) = self.tabs.get_mut(index).map(|tab| &mut tab.body) {
            chat.add_image(path);
        }
    }

    /// `--ui-probe agent-image-hold=`.
    pub fn probe_image_hold(&mut self, held: bool) {
        for tab in &mut self.tabs {
            if let Body::Chat(chat) = &mut tab.body {
                chat.images.set_held(held);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use musializer_core::agent::{Protocol, ToolState};
    use std::time::Duration;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "musializer-chat-images-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, b"bytes").unwrap();
            path
        }

        fn images(&self) -> ChatImages {
            let mut images = ChatImages::default();
            images.set_directory(Some(self.0.clone()));
            images
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn transcript() -> Transcript {
        Transcript::new(Protocol::Claude)
    }

    fn say(transcript: &mut Transcript, text: &str) {
        transcript.entries.push(Entry::Assistant {
            text: text.to_owned(),
        });
    }

    fn shown(transcript: &Transcript) -> Vec<String> {
        transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Image { path } => Some(file_name(Path::new(path))),
                _ => None,
            })
            .collect()
    }

    /// The first tick, which takes what is there as read; then the chat.
    fn started(images: &mut ChatImages, transcript: &mut Transcript) {
        images.scan(transcript);
    }

    /// Ticks until the disk thread has answered everything asked.
    fn settle(images: &mut ChatImages, transcript: &mut Transcript) {
        for _ in 0..5000 {
            images.scan(transcript);
            if images.unanswered == 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the disk thread never answered");
    }

    #[test]
    fn a_stuck_disk_holds_up_no_tick() {
        let scratch = Scratch::new("wedged");
        scratch.write("a.png");
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        let (release, wedged) = mpsc::channel();
        images.ask(Ask::Wedge(wedged));
        say(&mut t, "here is a.png");
        for _ in 0..50 {
            images.scan(&mut t);
        }
        assert!(shown(&t).is_empty(), "the answer has not come");
        assert_eq!(images.unanswered, 1);
        release.send(()).unwrap();
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
    }

    #[test]
    fn a_mention_on_the_heels_of_a_reopen_finds_it_seeded() {
        let scratch = Scratch::new("heels");
        let path = scratch.write("a.png");
        let mut t = transcript();
        t.entries.push(Entry::Image {
            path: path.display().to_string(),
        });
        let mut images = scratch.images();
        images.scan(&mut t);
        say(&mut t, "a.png again");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
    }

    #[test]
    fn a_tools_whole_arguments_are_read_past_its_chip() {
        let scratch = Scratch::new("long");
        let deep = scratch.0.join("d".repeat(120)).join("e".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let path = deep.join("my sheet.png");
        std::fs::write(&path, b"bytes").unwrap();
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        t.receive(
            &serde_json::json!({"type":"assistant","message":{"id":"m","content":[
                {"type":"tool_use","id":"u","name":"Read","input":{"file_path":path}}]}})
            .to_string(),
        );
        t.receive(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u","content":[{"type":"image"}],"is_error":false}]}}"#,
        );
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["my sheet.png"]);
    }

    #[test]
    fn dot_dot_goes_through_a_symlink_as_the_filesystem_does() {
        let scratch = Scratch::new("symlink");
        std::fs::create_dir_all(scratch.0.join("other/sub")).unwrap();
        std::fs::write(scratch.0.join("other/a.png"), b"the one").unwrap();
        std::os::unix::fs::symlink(scratch.0.join("other/sub"), scratch.0.join("link")).unwrap();
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        say(&mut t, "link/../a.png");
        settle(&mut images, &mut t);
        let Some(Entry::Image { path }) = t.entries.last() else {
            panic!("nothing shown");
        };
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"the one",
            "{path} must not lose its .."
        );
        // The same file by its other names is one picture.
        say(&mut t, "other/a.png");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t).len(), 1);
    }

    fn pixel() -> Result<Decoded, Failure> {
        Ok(Decoded {
            image: ColorImage::filled([1, 1], Color32::RED),
            natural: [1, 1],
        })
    }

    fn done(path: &Path, large: bool, generation: u64) -> Done {
        Done {
            path: path.to_path_buf(),
            large,
            generation,
            result: pixel(),
        }
    }

    #[test]
    fn only_the_newest_decode_of_a_thumbnail_counts() {
        let ctx = egui::Context::default();
        let path = Path::new("/s/a.png");
        let mut images = ChatImages::default();
        // A running, a rewrite, B asked for: A lands first.
        images.thumbs.insert(
            path.to_path_buf(),
            Thumb::Decoding {
                generation: 2,
                cancel: Arc::new(AtomicBool::new(false)),
                wanted: 0,
            },
        );
        images.finish(&ctx, done(path, false, 1));
        assert!(matches!(
            images.thumbs.get(path),
            Some(Thumb::Decoding { generation: 2, .. })
        ));
        assert!(!images.sizes.contains_key(path), "A's size is not B's");
        images.finish(&ctx, done(path, false, 2));
        assert!(matches!(images.thumbs.get(path), Some(Thumb::Ready { .. })));
    }

    #[test]
    fn a_decode_of_a_forgotten_thumbnail_is_dropped() {
        let ctx = egui::Context::default();
        let path = Path::new("/s/a.png");
        let mut images = ChatImages::default();
        images.finish(&ctx, done(path, false, 7));
        assert!(images.thumbs.is_empty());
    }

    #[test]
    fn only_the_newest_decode_of_the_large_view_counts() {
        let ctx = egui::Context::default();
        let path = Path::new("/s/a.png");
        let mut images = ChatImages::default();
        images.large = Some(Large {
            path: path.to_path_buf(),
            name: "a.png".into(),
            natural: [1, 1],
            thumbnail: ctx.load_texture(
                "t",
                ColorImage::filled([1, 1], Color32::RED),
                TextureOptions::LINEAR,
            ),
            full: None,
            generation: 2,
            cancel: Arc::new(AtomicBool::new(false)),
            bounds: [1, 1],
            failed: None,
            opening: false,
            return_focus: false,
        });
        images.finish(&ctx, done(path, true, 1));
        assert!(images.large.as_ref().unwrap().full.is_none());
        images.finish(&ctx, done(path, true, 2));
        assert!(images.large.as_ref().unwrap().full.is_some());
    }

    fn job(path: &Path, generation: u64) -> Job {
        Job {
            path: path.to_path_buf(),
            large: false,
            generation,
            cancel: Arc::new(AtomicBool::new(false)),
            bounds: [8, 8],
        }
    }

    fn wait_until(mut ready: impl FnMut() -> bool) {
        for _ in 0..5000 {
            if ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("never happened");
    }

    #[test]
    fn the_queues_are_bounded_and_the_worker_waits_for_room() {
        let scratch = Scratch::new("queues");
        let bad = scratch.write("bad.png");
        let decoder = Decoder::start().unwrap();
        for generation in 0..MOST_RESULTS as u64 {
            assert!(decoder.submit(job(&bad, generation)).is_ok());
        }
        wait_until(|| decoder.shared.lock().results.len() == MOST_RESULTS);
        // Nothing drains, so nothing more runs and the queue fills to its bound.
        for generation in 0..MOST_JOBS as u64 {
            assert!(decoder.submit(job(&bad, 100 + generation)).is_ok());
        }
        assert!(decoder.submit(job(&bad, 999)).is_err(), "full is full");
        std::thread::sleep(Duration::from_millis(50));
        {
            let queue = decoder.shared.lock();
            assert_eq!(queue.results.len(), MOST_RESULTS);
            assert_eq!(queue.jobs.len(), MOST_JOBS);
        }
        assert_eq!(decoder.take_results().len(), MOST_RESULTS);
        wait_until(|| decoder.shared.lock().results.len() == MOST_RESULTS);
        // Newest first: what was asked last is what is on screen.
        let first = decoder.take_results().pop_front().unwrap();
        assert_eq!(first.generation, 100 + MOST_JOBS as u64 - 1);
    }

    #[test]
    fn a_hidden_chat_keeps_no_decoding_and_no_pixels() {
        let scratch = Scratch::new("hidden");
        let bad = scratch.write("bad.png");
        let mut images = scratch.images();
        let mut t = transcript();
        images.scan(&mut t);
        images.request_thumb(bad.clone(), [8, 8]);
        assert!(images.thumbs.contains_key(&bad));
        images.scan(&mut t);
        images.scan(&mut t);
        assert!(images.thumbs.is_empty(), "asked again when it is seen");
        let decoder = images.decoder.as_ref().unwrap();
        wait_until(|| decoder.shared.lock().running.is_none());
        images.scan(&mut t);
        let queue = images.decoder.as_ref().unwrap().shared.lock();
        assert!(queue.jobs.is_empty() && queue.results.is_empty());
    }

    #[test]
    fn a_picture_scrolled_away_gives_up_its_place() {
        let scratch = Scratch::new("scrolled");
        let bad = scratch.write("bad.png");
        let mut images = scratch.images();
        images.frame = 1;
        images.request_thumb(bad.clone(), [8, 8]);
        images.frame = 1 + STALE_FRAMES;
        images.drop_scrolled_away();
        assert!(images.thumbs.contains_key(&bad), "not yet");
        images.frame += 1;
        images.drop_scrolled_away();
        assert!(!images.thumbs.contains_key(&bad));
    }

    #[test]
    fn dropping_a_chat_stops_its_worker() {
        let decoder = Decoder::start().unwrap();
        let shared = decoder.shared.clone();
        drop(decoder);
        assert!(shared.lock().finished);
    }

    #[test]
    fn a_mention_shows_once_while_the_file_stands_still() {
        let scratch = Scratch::new("once");
        scratch.write("a.png");
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        say(&mut t, "here is a.png");
        settle(&mut images, &mut t);
        say(&mut t, "a.png again, and ./a.png");
        settle(&mut images, &mut t);
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
    }

    #[test]
    fn a_rewritten_file_shows_again() {
        let scratch = Scratch::new("rewrite");
        let path = scratch.write("a.png");
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        say(&mut t, "a.png");
        settle(&mut images, &mut t);
        let later = SystemTime::now() + Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        say(&mut t, "a.png");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png", "a.png"]);
    }

    #[test]
    fn a_name_with_no_regular_file_behind_it_is_not_a_picture() {
        let scratch = Scratch::new("absent");
        std::fs::create_dir(scratch.0.join("folder.png")).unwrap();
        std::fs::write(scratch.0.join("empty.png"), b"").unwrap();
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        say(
            &mut t,
            "I will write later.png, see folder.png and empty.png",
        );
        settle(&mut images, &mut t);
        assert!(shown(&t).is_empty());
    }

    #[test]
    fn a_tool_is_read_once_it_has_returned_and_its_detail_with_it() {
        let scratch = Scratch::new("tool");
        scratch.write("a.png");
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        t.entries.push(Entry::Tool {
            id: "t".into(),
            name: "Bash".into(),
            detail: "render --out a.png".into(),
            state: ToolState::Running,
            output: String::new(),
        });
        settle(&mut images, &mut t);
        assert!(shown(&t).is_empty(), "the file may not be written yet");
        if let Entry::Tool { state, .. } = &mut t.entries[0] {
            *state = ToolState::Done;
        }
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
    }

    #[test]
    fn a_running_tool_does_not_hold_back_what_settled_after_it() {
        let scratch = Scratch::new("order");
        scratch.write("a.png");
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        t.entries.push(Entry::Tool {
            id: "t".into(),
            name: "Bash".into(),
            detail: "sleep 100".into(),
            state: ToolState::Running,
            output: String::new(),
        });
        say(&mut t, "meanwhile a.png");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
    }

    #[test]
    fn a_listing_does_not_fill_the_chat() {
        let scratch = Scratch::new("listing");
        let names: Vec<String> = (0..9).map(|i| format!("s{i}.png")).collect();
        for name in &names {
            scratch.write(name);
        }
        let mut images = scratch.images();
        let mut t = transcript();
        started(&mut images, &mut t);
        say(&mut t, &names.join(" "));
        settle(&mut images, &mut t);
        assert_eq!(shown(&t).len(), MOST_FROM_ONE_ENTRY);
        assert!(t.entries.iter().any(
            |entry| matches!(entry, Entry::Notice { text, .. } if text == "Showing 6 of 9 pictures named here")
        ));
        // One past the limit was left unseen, so naming it shows it.
        say(&mut t, "s8.png");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t).last().map(String::as_str), Some("s8.png"));
    }

    #[test]
    fn a_reopened_chat_is_not_read_as_news() {
        let scratch = Scratch::new("reopen");
        let path = scratch.write("a.png");
        scratch.write("b.png");
        let mut t = transcript();
        say(&mut t, "a.png and b.png");
        t.entries.push(Entry::Image {
            path: path.display().to_string(),
        });
        let mut images = scratch.images();
        settle(&mut images, &mut t);
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png"]);
        say(&mut t, "a.png once more");
        settle(&mut images, &mut t);
        assert_eq!(
            shown(&t),
            ["a.png"],
            "seeded by what the chat already shows"
        );
        say(&mut t, "b.png now");
        settle(&mut images, &mut t);
        assert_eq!(shown(&t), ["a.png", "b.png"]);
    }

    #[test]
    fn paths_resolve_against_the_chat_and_clean_up() {
        let directory = Path::new("/work/song");
        assert_eq!(
            resolve(Some(directory), "a.png"),
            Path::new("/work/song/a.png")
        );
        assert_eq!(
            resolve(Some(directory), "./x/a.png"),
            Path::new("/work/song/x/a.png")
        );
        assert_eq!(
            resolve(Some(directory), "../a.png"),
            Path::new("/work/song/../a.png"),
            "only the filesystem knows what .. is"
        );
        assert_eq!(
            resolve(Some(directory), "/tmp/a.png"),
            Path::new("/tmp/a.png")
        );
    }

    #[test]
    fn a_relative_directory_gives_an_absolute_entry() {
        let mut images = ChatImages::default();
        images.set_directory(Some(PathBuf::from("build/real")));
        let directory = images.directory.clone().unwrap();
        assert!(directory.is_absolute());
        assert_eq!(
            resolve(Some(&directory), "a.png"),
            std::env::current_dir().unwrap().join("build/real/a.png")
        );
    }

    #[test]
    fn sizes_keep_the_shape_and_a_thumbnail_never_magnifies() {
        assert_eq!(shrunk([1920, 1080], [292, 360]), [292, 164]);
        assert_eq!(shrunk([1080, 1920], [292, 360]), [203, 360]);
        assert_eq!(shrunk([64, 64], [292, 360]), [64, 64]);
        assert_eq!(shrunk([9000, 1], [292, 360]), [292, 1]);
        assert_eq!(
            thumbnail_size([1920, 1080], 292.0, 1.0),
            egui::vec2(292.0, 164.25)
        );
        assert_eq!(thumbnail_size([64, 64], 292.0, 1.0), egui::vec2(64.0, 64.0));
        assert_eq!(thumbnail_size([64, 64], 292.0, 2.0), egui::vec2(32.0, 32.0));
        assert_eq!(
            fitted([640, 360], egui::vec2(1280.0, 1280.0)),
            egui::vec2(1280.0, 720.0)
        );
    }

    #[test]
    fn shrinking_keeps_an_opaque_colour_exact() {
        let pixels: Vec<u8> = [17, 128, 201, 255].repeat(70 * 30);
        let image = shrink(&pixels, [70, 30], [13, 7]);
        assert_eq!(image.size, [13, 7]);
        assert!(image
            .pixels
            .iter()
            .all(|pixel| *pixel == Color32::from_rgb(17, 128, 201)));
    }

    #[test]
    fn shrinking_does_not_bleed_a_transparent_pixels_hidden_colour() {
        // White that is not there beside opaque black: the average is black
        // at half alpha. Averaging straight colour would make it grey.
        let pixels = [255, 255, 255, 0, 0, 0, 0, 255];
        let image = shrink(&pixels, [2, 1], [1, 1]);
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_premultiplied(0, 0, 0, 128)
        );
    }

    #[test]
    fn eviction_drops_the_oldest_and_keeps_what_is_on_screen() {
        let ctx = egui::Context::default();
        let mut images = ChatImages {
            frame: 1000,
            ..ChatImages::default()
        };
        let thumb = |last_drawn| Thumb::Ready {
            texture: ctx.load_texture(
                "t",
                ColorImage::filled([2, 2], Color32::RED),
                TextureOptions::LINEAR,
            ),
            natural: [2, 2],
            last_drawn,
        };
        for index in 0..MOST_THUMBS as u64 + 3 {
            images
                .thumbs
                .insert(PathBuf::from(format!("{index}.png")), thumb(index));
        }
        images.evict();
        assert_eq!(images.thumbs.len(), MOST_THUMBS);
        for gone in ["0.png", "1.png", "2.png"] {
            assert!(!images.thumbs.contains_key(Path::new(gone)));
        }
        // All on screen, so all stay, however many that is.
        images.thumbs.clear();
        for index in 0..MOST_THUMBS as u64 + 3 {
            images
                .thumbs
                .insert(PathBuf::from(format!("{index}.png")), thumb(1000));
        }
        images.evict();
        assert_eq!(images.thumbs.len(), MOST_THUMBS + 3);
    }

    #[test]
    fn a_failure_says_which_file() {
        assert_eq!(Failure::Missing.says("a.png"), "a.png is missing");
        assert_eq!(
            Failure::Undecodable.says("a.png"),
            "a.png could not be read as a picture"
        );
    }
}
