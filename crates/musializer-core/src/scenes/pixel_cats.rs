//! Pixel Cats: a group of pixel-art stagings, each its own scene.
//!
//! Post-legacy (2026-10-24), no oracle. The operator was shown a music video a
//! model had made for his chiptune-gabber track with nothing but Python, numpy
//! and ffmpeg: every frame drawn at 320x180 with a hand-made 5x7 font and
//! procedural cats, upscaled six times, and every section a staged gag tied to
//! what its lyric says. This brings that craft in as scenes, so a treatment can
//! make a video like it for any song, deterministically.
//!
//! **One implementation, six scenes.** Each staging — boot screen, root shell,
//! side-scroller, barbershop, rose window, gabber tunnel — is its own
//! [`SceneId`] with its own token (`pixel-boot` …), its own tile in the
//! picker's *Pixel cats* family and its own settings rows, so a treatment
//! names it directly (`"scene": "pixel-rave"`). They share this module, one
//! state type and one settings shape (`settings::index::pixel`), which is what
//! keeps six scenes one idea rather than six drifting copies.
//!
//! **The frame is drawn on the CPU.** A staging draws into a [`raster::Raster`]
//! a few hundred pixels across — about 180 on the short axis, the integer
//! factor of the output that lands nearest ([`canvas_size`]) — and the drawing
//! half uploads it and scales it up nearest-neighbour behind an optional CRT
//! grid. At that size a whole frame is ~58 000 pixels, which is less work than
//! the draw calls it would take to put the same picture through raylib, and it
//! buys three things a GPU path cannot: the frame is the same bytes on NVIDIA
//! and on llvmpipe (the gate's rasterizer split cannot reach it), every
//! staging's response to the music is checkable in a unit test by counting
//! pixels, and preview, still, clip and export agree because they are one pure
//! function of the state and the frame.
//!
//! **State is followers and events, replayed.** [`PixelCatsState::update`]
//! counts beats, detects kicks and hi-hat ticks, keeps the recent onsets that
//! drive word pops and jumps, smooths a vocal and a loudness level, integrates
//! the scrolling and remembers the lines already sung (the shell's history).
//! A still replays it from the start like every stateful scene's. Drawing
//! never advances it.
//!
//! **No strobing.** A hit shakes, punches in and splits the colour; nothing
//! here flashes the whole frame (the source video's own rule).

pub mod font;
pub mod raster;
pub mod sprites;
mod stagings;

use std::any::Any;
use std::collections::VecDeque;

use crate::scene::settings::index::pixel as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use raster::{Raster, Rgb};

/// Which picture a Pixel Cats scene stages its line as. One per [`SceneId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Staging {
    /// A handheld's boot screen: the line drops in as the logo, an ASCII cat
    /// assembles on the beat.
    Boot,
    /// A lyric editor in a terminal: every line typed behind a prompt, the
    /// music writing a stage direction above it that restyles it.
    Shell,
    /// A side-scroller over keyboard-key rooftops: the cat runs on the line's
    /// own letters and jumps on the hi-hats.
    Runner,
    /// A barbershop quartet of cats on a curtained stage with a drum kit.
    Quartet,
    /// Monk cats under a rose window, the line laid out on a chant staff.
    Chapel,
    /// A gabber tunnel with a giant cat face; the sung word pops on onsets.
    Rave,
}

impl Staging {
    pub const ALL: [Self; 6] = [
        Self::Boot,
        Self::Shell,
        Self::Runner,
        Self::Quartet,
        Self::Chapel,
        Self::Rave,
    ];

    #[must_use]
    pub fn scene(self) -> SceneId {
        match self {
            Self::Boot => SceneId::PixelBoot,
            Self::Shell => SceneId::PixelShell,
            Self::Runner => SceneId::PixelRunner,
            Self::Quartet => SceneId::PixelQuartet,
            Self::Chapel => SceneId::PixelChapel,
            Self::Rave => SceneId::PixelRave,
        }
    }

    #[must_use]
    pub fn of(scene: SceneId) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|staging| staging.scene() == scene)
    }

    /// The report's name for it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Shell => "shell",
            Self::Runner => "runner",
            Self::Quartet => "quartet",
            Self::Chapel => "chapel",
            Self::Rave => "rave",
        }
    }
}

/// The colours the finished frame is mapped to (`settings.pixel-*.palette`).
/// The numeric value is persisted: append, never reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    /// Four greens, the handheld look.
    Handheld = 0,
    /// The staging's own full colour.
    Colour = 1,
    /// Four warm greys.
    Pocket = 2,
    /// Black, magenta, cyan and white.
    Arcade = 3,
    /// Four reds on black.
    Virtual = 4,
    /// Violet through rose to cream.
    Dusk = 5,
}

impl Palette {
    pub const ALL: [Self; 6] = [
        Self::Handheld,
        Self::Colour,
        Self::Pocket,
        Self::Arcade,
        Self::Virtual,
        Self::Dusk,
    ];

    #[must_use]
    pub fn from_setting(value: f32) -> Self {
        let index = if value.is_finite() {
            value.round().max(0.0) as usize
        } else {
            1
        };
        Self::ALL.get(index).copied().unwrap_or(Self::Colour)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Handheld => "handheld",
            Self::Colour => "colour",
            Self::Pocket => "pocket",
            Self::Arcade => "arcade",
            Self::Virtual => "virtual",
            Self::Dusk => "dusk",
        }
    }

    /// The four shades a luma quantizer maps to, darkest first; `None` for full
    /// colour.
    #[must_use]
    pub fn ramp(self) -> Option<[Rgb; 4]> {
        Some(match self {
            Self::Handheld => [
                Rgb::hex(0x10301c),
                Rgb::hex(0x34623a),
                Rgb::hex(0x8aa832),
                Rgb::hex(0xc4d67c),
            ],
            Self::Colour => return None,
            Self::Pocket => [
                Rgb::hex(0x1e1c1a),
                Rgb::hex(0x5a5650),
                Rgb::hex(0xa29d92),
                Rgb::hex(0xe2ddd0),
            ],
            Self::Arcade => [
                Rgb::hex(0x0a0610),
                Rgb::hex(0xc6328a),
                Rgb::hex(0x36c8d8),
                Rgb::hex(0xf4f0fa),
            ],
            Self::Virtual => [
                Rgb::hex(0x080002),
                Rgb::hex(0x58000e),
                Rgb::hex(0xb2081e),
                Rgb::hex(0xff3a44),
            ],
            Self::Dusk => [
                Rgb::hex(0x1c1030),
                Rgb::hex(0x6a2c6c),
                Rgb::hex(0xd8706c),
                Rgb::hex(0xf8dcae),
            ],
        })
    }
}

/// The internal canvas for an output `width` by `height`: its size in pixels
/// and the integer factor each of its pixels is drawn at.
///
/// The factor is the whole number that puts the short axis nearest `target`
/// (180 by default), so 1280x720, 1920x1080 and a supersampled 3840x2160 all
/// draw the same 320x180 picture and a 1080x1920 portrait draws 180x320. The
/// canvas then covers the output to within one factor's worth of pixels on
/// each axis; the drawing half centres it and the remainder is the bezel.
#[must_use]
pub fn canvas_size(width: f32, height: f32, target: f32) -> (i32, i32, i32) {
    if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
        return (16, 16, 1);
    }
    let target = if target.is_finite() {
        target.clamp(60.0, 1080.0)
    } else {
        180.0
    };
    let short = width.min(height);
    let factor = (short / target).round().max(1.0) as i32;
    let cw = ((width / factor as f32).floor() as i32).clamp(16, MAX_CANVAS);
    let ch = ((height / factor as f32).floor() as i32).clamp(16, MAX_CANVAS);
    (cw, ch, factor)
}

/// The largest canvas edge: the drawing half uploads into a texture this big,
/// allocated once.
pub const MAX_CANVAS: i32 = 1024;

// -- deterministic noise -------------------------------------------------------

/// SplitMix64's finalizer: the one hash every random-looking choice here goes
/// through, keyed on the seed and on track time — never on a frame counter or
/// the wall clock, so two renders of one moment agree at any frame rate.
#[must_use]
pub fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// A uniform value in `0..1` from a key and a salt.
#[must_use]
pub fn unit(key: u64, salt: u64) -> f32 {
    (mix(key ^ salt.wrapping_mul(0x2545_f491_4f6c_dd1d)) >> 40) as f32 / 16_777_216.0
}

/// Seconds in `0..1` as a smoothstep, for eases that land.
#[must_use]
pub fn smooth(value: f32) -> f32 {
    let v = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    v * v * (3.0 - 2.0 * v)
}

/// A cubic ease-out in `0..1`.
#[must_use]
pub fn ease_out(value: f32) -> f32 {
    let v = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    1.0 - (1.0 - v).powi(3)
}

// -- state ---------------------------------------------------------------------

/// Bass-transient gate on the mean positive excursion of the lowest quarter of
/// the bands over their trails: Clawd's measured kick detector, shared rather
/// than re-derived (`clawd::KICK_THRESHOLD` documents the measurement).
const KICK_THRESHOLD: f32 = crate::scenes::clawd::KICK_THRESHOLD;
/// A 16th at 180 BPM is 83 ms; a kick closer than this is one push.
const KICK_REFRACTORY: f32 = 0.11;
/// The same detector over the top quarter of the bands: hi-hats and the bright
/// edges of a vocal. Measured on the meow track (2026-10-24): 0.05 tracks the
/// off-beat hats of its gabber sections and stays quiet in the music box.
const TICK_THRESHOLD: f32 = 0.05;
const TICK_REFRACTORY: f32 = 0.09;
const ONSET_REFRACTORY: f32 = 0.10;
/// How long an event stays in the recent lists, in seconds.
const EVENT_MEMORY: f64 = 3.0;
/// How many sung lines the shell remembers.
const HISTORY: usize = 8;

/// A line already sung, as the shell shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sung {
    pub id: u64,
    pub text: String,
    pub tag: Option<String>,
}

/// The stage directions the shell restyles a line for. A tag the music
/// writes is one of the first four; `ROOT` answers a line that asks for it; a
/// line may open with any tag of its own, and an unknown one is shown as
/// written over plain type (one containing `KEY` turns the type cyan and lifts
/// the page, a key change). A line that says `logout`, `exit` or `goodbye`
/// shuts the session down.
pub const TAGS: [&str; 6] = [
    "BELTED",
    "WHISPERED",
    "FALSETTO",
    "GABBER",
    "ROOT",
    "LOGOUT",
];

/// Everything one Pixel Cats scene remembers between frames.
#[derive(Clone, Debug, PartialEq)]
pub struct PixelCatsState {
    staging: Staging,
    seed: u64,
    /// Seconds this instance has run, on the scene clock.
    age: f32,
    beats: u64,
    last_phase: f32,
    kick: f32,
    kicks: u64,
    kick_wait: f32,
    ticks: u64,
    tick_wait: f32,
    onsets_seen: u64,
    onset_wait: f32,
    was_onset: bool,
    vocal: f32,
    energy: f32,
    travel: f64,
    kick_times: VecDeque<f64>,
    tick_times: VecDeque<f64>,
    onset_times: VecDeque<f64>,
    history: VecDeque<Sung>,
    current: Option<Sung>,
    /// Seconds since a line was last on screen; zero while one is.
    quiet: f32,
    dynamics: bool,
}

impl PixelCatsState {
    #[must_use]
    pub fn new(staging: Staging, seed: u64) -> Self {
        Self {
            staging,
            seed,
            age: 0.0,
            beats: 0,
            last_phase: 0.0,
            kick: 0.0,
            kicks: 0,
            kick_wait: 0.0,
            ticks: 0,
            tick_wait: 0.0,
            onsets_seen: 0,
            onset_wait: 0.0,
            was_onset: false,
            vocal: 0.0,
            energy: 0.0,
            travel: 0.0,
            kick_times: VecDeque::new(),
            tick_times: VecDeque::new(),
            onset_times: VecDeque::new(),
            history: VecDeque::new(),
            current: None,
            quiet: 0.0,
            dynamics: false,
        }
    }

    #[must_use]
    pub fn staging(&self) -> Staging {
        self.staging
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn age(&self) -> f32 {
        self.age
    }

    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kicks
    }

    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    #[must_use]
    pub fn beats(&self) -> u64 {
        self.beats
    }

    #[must_use]
    pub fn onsets(&self) -> u64 {
        self.onsets_seen
    }

    /// The kick envelope: jumps on a kick, decays in about a sixth of a second.
    #[must_use]
    pub fn kick(&self) -> f32 {
        self.kick
    }

    #[must_use]
    pub fn vocal(&self) -> f32 {
        self.vocal
    }

    #[must_use]
    pub fn energy(&self) -> f32 {
        self.energy
    }

    #[must_use]
    pub fn travel(&self) -> f64 {
        self.travel
    }

    /// Seconds since a line was last on screen (zero while one is, and
    /// counted from the scene's start before the first).
    #[must_use]
    pub fn quiet(&self) -> f32 {
        self.quiet
    }

    /// Lines sung while this scene ran, oldest first.
    pub fn history(&self) -> impl Iterator<Item = &Sung> {
        self.history.iter()
    }

    /// The tag the current line was given when it started.
    #[must_use]
    pub fn current_tag(&self, line: u64) -> Option<String> {
        self.current
            .as_ref()
            .filter(|current| current.id == line)
            .and_then(|current| current.tag.clone())
    }

    fn band_excursion(bands: &[f32], trails: &[f32], top: bool) -> f32 {
        let count = bands.len().min(trails.len());
        if count == 0 {
            return 0.0;
        }
        let quarter = (count / 4).max(1);
        let range = if top {
            count - quarter..count
        } else {
            0..quarter
        };
        let sum: f32 = range
            .clone()
            .map(|i| (bands[i] - trails[i]).max(0.0))
            .filter(|v| v.is_finite())
            .sum();
        sum / quarter as f32
    }

    fn remember(list: &mut VecDeque<f64>, time: f64) {
        list.push_back(time);
        while list.len() > 64 {
            list.pop_front();
        }
    }

    fn forget(list: &mut VecDeque<f64>, now: f64) {
        while list
            .front()
            .is_some_and(|&t| t < now - EVENT_MEMORY || t > now)
        {
            list.pop_front();
        }
    }

    /// The tag a new line gets: the line's own leading `[TAG]` if it has one;
    /// otherwise what the music is doing as it starts — loud is belted, quiet
    /// is whispered, a bright mix is falsetto, a kick storm is gabber — and a
    /// line that asks for `sudo` or `root` is run as root.
    fn tag_for(&self, text: &str, response: f32, balance: f32) -> Option<String> {
        if let Some(tag) = leading_tag(text) {
            return Some(tag);
        }
        let folded = font::folded(text);
        let words: Vec<&str> = folded
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        if words
            .iter()
            .any(|word| matches!(*word, "LOGOUT" | "EXIT" | "SHUTDOWN" | "GOODBYE" | "BYE"))
            || folded.contains("LOG OUT")
        {
            return Some("LOGOUT".into());
        }
        if words.iter().any(|word| *word == "SUDO" || *word == "ROOT") {
            return Some("ROOT".into());
        }
        if response <= 0.0 {
            return None;
        }
        let recent_kicks = self.kick_times.len();
        let tag = if self.energy >= 0.80 && recent_kicks >= 6 {
            "GABBER"
        } else if self.energy >= 0.70 {
            "BELTED"
        } else if self.energy <= 0.30 {
            "WHISPERED"
        } else if balance >= 0.42 {
            "FALSETTO"
        } else {
            return None;
        };
        Some(tag.into())
    }
}

/// A line's leading `[TAG]`, folded, if it opens with one.
#[must_use]
pub fn leading_tag(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let end = rest.find(']')?;
    let tag = font::folded(rest[..end].trim());
    (!tag.is_empty() && tag.len() <= 16).then_some(tag)
}

/// The line without its leading `[TAG]`.
#[must_use]
pub fn without_tag(text: &str) -> &str {
    let trimmed = text.trim_start();
    if leading_tag(trimmed).is_some() {
        if let Some(end) = trimmed.find(']') {
            return trimmed[end + 1..].trim();
        }
    }
    trimmed.trim()
}

impl SceneState for PixelCatsState {
    fn id(&self) -> SceneId {
        self.staging.scene()
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        let dt = frame.delta_seconds;
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let scene = self.id();
        let response = frame.setting(scene, control::RESPONSE).max(0.0);
        let motion = frame.setting(scene, control::MOTION).max(0.0);
        let now = frame.time_seconds;
        let audio = &frame.audio;
        self.age += dt;
        self.dynamics = frame.dynamics.is_some();

        // Beats: a wrap of the tracker's phase.
        let phase = audio.beat_phase.clamp(0.0, 1.0);
        if self.last_phase - phase > 0.5 {
            self.beats = self.beats.wrapping_add(1);
        }
        self.last_phase = phase;

        let kick = Self::band_excursion(audio.bands, audio.trails, false);
        self.kick_wait = (self.kick_wait - dt).max(0.0);
        if kick >= KICK_THRESHOLD && self.kick_wait <= 0.0 {
            self.kick_wait = KICK_REFRACTORY;
            self.kicks = self.kicks.wrapping_add(1);
            Self::remember(&mut self.kick_times, now);
            let power = (kick / (2.5 * KICK_THRESHOLD)).clamp(0.45, 1.2);
            self.kick = self.kick.max(power);
        }
        self.kick *= (-dt * 6.5).exp();

        let tick = Self::band_excursion(audio.bands, audio.trails, true);
        self.tick_wait = (self.tick_wait - dt).max(0.0);
        if tick >= TICK_THRESHOLD && self.tick_wait <= 0.0 {
            self.tick_wait = TICK_REFRACTORY;
            self.ticks = self.ticks.wrapping_add(1);
            Self::remember(&mut self.tick_times, now);
        }

        self.onset_wait = (self.onset_wait - dt).max(0.0);
        if audio.onset && !self.was_onset && self.onset_wait <= 0.0 {
            self.onset_wait = ONSET_REFRACTORY;
            self.onsets_seen = self.onsets_seen.wrapping_add(1);
            Self::remember(&mut self.onset_times, now);
        }
        self.was_onset = audio.onset;
        for list in [
            &mut self.kick_times,
            &mut self.tick_times,
            &mut self.onset_times,
        ] {
            Self::forget(list, now);
        }

        let (_, mids, treble) = audio.spectral_regions();
        let follow = |current: f32, target: f32, up: f32, down: f32| {
            let target = if target.is_finite() {
                target.clamp(0.0, 1.5)
            } else {
                0.0
            };
            let rate = if target > current { up } else { down };
            current + (target - current) * (1.0 - (-rate * dt).exp())
        };
        self.vocal = follow(self.vocal, mids * 1.6, 14.0, 4.0);
        let level = frame
            .dynamics
            .map_or((audio.rms * 1.8).clamp(0.0, 1.0), |profile| {
                profile.level(audio.rms)
            });
        self.energy = follow(self.energy, level, 6.0, 1.6);
        self.travel += f64::from(dt)
            * f64::from(motion)
            * f64::from(0.7 + response * (0.9 * self.energy + 0.8 * self.kick));

        // The shell's memory of what has been sung.
        let lyric = frame
            .lyric
            .filter(|lyric| !without_tag(&lyric.text).is_empty());
        if lyric.is_some() {
            self.quiet = 0.0;
        } else {
            self.quiet += dt;
        }
        let changed = match (lyric, &self.current) {
            (Some(lyric), Some(current)) => lyric.id != current.id,
            (None, None) => false,
            _ => true,
        };
        if changed {
            if let Some(previous) = self.current.take() {
                self.history.push_back(previous);
                while self.history.len() > HISTORY {
                    self.history.pop_front();
                }
            }
            if let Some(lyric) = lyric {
                let balance = treble / (mids + treble + 0.001);
                self.current = Some(Sung {
                    id: lyric.id,
                    text: font::folded(without_tag(&lyric.text)),
                    tag: self.tag_for(&lyric.text, response, balance),
                });
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor(staging: Staging) -> SceneDescriptor {
    SceneDescriptor {
        id: staging.scene(),
        state_version: 1,
        make_state: match staging {
            Staging::Boot => |seed| Box::new(PixelCatsState::new(Staging::Boot, seed)),
            Staging::Shell => |seed| Box::new(PixelCatsState::new(Staging::Shell, seed)),
            Staging::Runner => |seed| Box::new(PixelCatsState::new(Staging::Runner, seed)),
            Staging::Quartet => |seed| Box::new(PixelCatsState::new(Staging::Quartet, seed)),
            Staging::Chapel => |seed| Box::new(PixelCatsState::new(Staging::Chapel, seed)),
            Staging::Rave => |seed| Box::new(PixelCatsState::new(Staging::Rave, seed)),
        },
    }
}

// -- the moment ----------------------------------------------------------------

/// The lyric line on screen, as a staging reads it.
#[derive(Clone, Debug)]
pub struct Line {
    pub id: u64,
    /// Folded, without a leading tag.
    pub text: String,
    /// The shell's stage direction, if the line was given one.
    pub tag: Option<String>,
    pub elapsed: f32,
    pub duration: f32,
    /// Characters a typewriter has put down, of [`Self::total`].
    pub typed: usize,
    pub total: usize,
    /// 0 at the cue's start, 1 once the whole line has landed.
    pub reveal: f32,
    /// Seconds into the cue at which the whole line has landed.
    pub revealed_at: f32,
    /// Character ranges of the words, in the folded text.
    pub words: Vec<(usize, usize)>,
    /// The word being sung now.
    pub sung: usize,
    /// Seconds since that word began.
    pub sung_since: f32,
    /// How far through the cue, 0..1.
    pub position: f32,
}

impl Line {
    /// Builds the line for a cue at `time`, typed at `pace`.
    ///
    /// The typewriter sets about 22 characters a second at pace 1 and never
    /// takes more than the first 55 % of the cue, so a still from the back half
    /// of any line shows all of it. The sung word walks the cue's first 90 %
    /// in proportion to each word's length: an estimate over the line's span,
    /// like Cadence's, never a measured word time.
    #[must_use]
    pub fn new(id: u64, text: &str, start: f64, end: f64, time: f64, pace: f32) -> Self {
        let text = font::folded(without_tag(text));
        let duration = (end - start).max(0.05) as f32;
        let elapsed = (time - start).max(0.0) as f32;
        let total = text.chars().count();
        let rate = 22.0 * pace.clamp(0.1, 10.0);
        let reveal_seconds = (total as f32 / rate).min(duration * 0.55).max(0.05);
        let reveal = (elapsed / reveal_seconds).clamp(0.0, 1.0);
        let typed = if reveal >= 1.0 {
            total
        } else {
            ((total as f32) * reveal).floor() as usize
        };
        let mut words = Vec::new();
        let mut start_char = None;
        for (index, c) in text.chars().chain(std::iter::once(' ')).enumerate() {
            match (c == ' ', start_char) {
                (false, None) => start_char = Some(index),
                (true, Some(begin)) => {
                    words.push((begin, index));
                    start_char = None;
                }
                _ => {}
            }
        }
        let position = (elapsed / duration).clamp(0.0, 1.0);
        let weights: Vec<f32> = words.iter().map(|&(a, b)| (b - a) as f32 + 1.5).collect();
        let sum: f32 = weights.iter().sum::<f32>().max(0.001);
        let mut cursor = 0.0;
        let mut sung = 0;
        let mut sung_start = 0.0;
        let walk = position / 0.9;
        for (index, weight) in weights.iter().enumerate() {
            if walk >= cursor / sum {
                sung = index;
                sung_start = cursor / sum * 0.9 * duration;
            }
            cursor += weight;
        }
        let sung_since = (elapsed - sung_start).max(0.0);
        Self {
            id,
            text,
            tag: None,
            elapsed,
            duration,
            typed,
            total,
            reveal,
            revealed_at: reveal_seconds,
            words,
            sung,
            sung_since,
            position,
        }
    }

    #[must_use]
    pub fn complete(&self) -> bool {
        self.typed >= self.total
    }

    /// The word being sung, as text.
    #[must_use]
    pub fn sung_word(&self) -> &str {
        self.word(self.sung)
    }

    #[must_use]
    pub fn word(&self, index: usize) -> &str {
        self.words.get(index).map_or("", |&(a, b)| {
            let start = self.text.char_indices().nth(a).map_or(0, |(i, _)| i);
            let end = self
                .text
                .char_indices()
                .nth(b)
                .map_or(self.text.len(), |(i, _)| i);
            &self.text[start..end]
        })
    }

    /// The first `typed` characters of a wrapped row.
    #[must_use]
    pub fn typed_part(&self, row: &font::Row) -> String {
        let shown = self.typed.saturating_sub(row.start);
        row.text.chars().take(shown).collect()
    }
}

/// One recent event, for a staging to animate: how long ago it happened and a
/// stable key to hash its look from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub age: f32,
    pub key: u64,
}

/// Everything a staging may read about this frame. Built once, so six
/// stagings cannot each read the music a slightly different way.
#[derive(Clone, Debug)]
pub struct Moment {
    pub staging: Staging,
    pub seed: u64,
    /// Track time in seconds.
    pub time: f32,
    /// Track time scaled by `motion`, for idle animation.
    pub clock: f32,
    pub duration: f32,
    pub width: i32,
    pub height: i32,
    /// 0..1 through the beat, and the beat's index in this scene.
    pub phase: f32,
    pub beat: u64,
    /// 1 on the beat, decaying through it.
    pub pulse: f32,
    /// Audio couplings, each already multiplied by `response`.
    pub kick: f32,
    pub vocal: f32,
    pub energy: f32,
    pub bass: f32,
    pub treble: f32,
    /// The band levels, for a staging that draws a meter.
    pub bands: Vec<f32>,
    pub response: f32,
    pub pops: bool,
    pub kicks: Vec<Event>,
    pub ticks: Vec<Event>,
    pub onsets: Vec<Event>,
    /// Onsets since the scene started (zero with no response).
    pub onset_count: u64,
    /// Scrolling distance, integrated in the state.
    pub travel: f32,
    pub line: Option<Line>,
    /// The voice's hue in degrees, when the line has one.
    pub voice_hue: Option<f32>,
    pub age: f32,
    pub motion: f32,
    pub history: Vec<Sung>,
    /// Seconds since a line was last on screen.
    pub quiet: f32,
    pub dynamics: bool,
}

impl Moment {
    /// The staging's accent hue: the line's voice when it has one, else
    /// `default` (MV9-h: a voice is one colour in every scene).
    #[must_use]
    pub fn accent_hue(&self, default: f32) -> f32 {
        self.voice_hue.unwrap_or(default)
    }

    /// `n` short-axis pixels at this canvas's size, relative to 180.
    #[must_use]
    pub fn px(&self, n: f32) -> i32 {
        (n * self.width.min(self.height) as f32 / 180.0).round() as i32
    }

    /// The integer sprite scale that reads well at this canvas size.
    #[must_use]
    pub fn sprite_scale(&self, at_180: i32) -> i32 {
        ((at_180 as f32 * self.width.min(self.height) as f32 / 180.0).round() as i32).max(1)
    }
}

fn events(list: &VecDeque<f64>, now: f64, seed: u64) -> Vec<Event> {
    list.iter()
        .filter(|&&t| t <= now)
        .map(|&t| Event {
            age: (now - t) as f32,
            key: mix(seed ^ (t * 1000.0).round() as u64),
        })
        .collect()
}

// -- the report ----------------------------------------------------------------

/// What one Pixel Cats frame drew, for the `pixel cats:` report line.
///
/// A staging with no line and one whose typewriter never started are the same
/// picture; so are a floor that should hold cats and an empty one, and a
/// palette that silently fell back to full colour. This says which.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelReport {
    pub staging: &'static str,
    pub palette: &'static str,
    pub canvas: (i32, i32, i32),
    /// `none`, `typing N/M` or `complete N`.
    pub line: String,
    pub glyphs: usize,
    pub cats: usize,
    pub pops: usize,
    pub beats: u64,
    pub kicks: u64,
    pub ticks: u64,
    pub onsets: u64,
    pub kick: f32,
    pub energy: f32,
    pub dynamics: bool,
    pub tag: Option<String>,
    pub shake: (i32, i32),
    pub split: i32,
    pub zoom: f32,
    pub glitch_rows: usize,
    pub power: &'static str,
    /// A staging's own evidence.
    pub detail: String,
}

impl PixelReport {
    #[must_use]
    pub fn describe(&self) -> String {
        if self.staging.is_empty() {
            return "none".into();
        }
        format!(
            "staging={} palette={} canvas={}x{}x{} line={} tag={} glyphs={} cats={} pops={} \
             beats={} kicks={} ticks={} onsets={} kick={:.2} energy={:.2} dyn={} \
             hit=shake({},{})+split{}+zoom{:.3} glitch={} power={} {}",
            self.staging,
            self.palette,
            self.canvas.0,
            self.canvas.1,
            self.canvas.2,
            self.line,
            self.tag.as_deref().unwrap_or("none"),
            self.glyphs,
            self.cats,
            self.pops,
            self.beats,
            self.kicks,
            self.ticks,
            self.onsets,
            self.kick,
            self.energy,
            if self.dynamics { "track" } else { "none" },
            self.shake.0,
            self.shake.1,
            self.split,
            self.zoom,
            self.glitch_rows,
            self.power,
            self.detail
        )
    }
}

/// Post effects a staging asks the compositor for, on top of the shared ones.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask {
    /// Extra shake, in canvas pixels at full kick.
    pub shake: f32,
    /// Extra colour split, in pixels.
    pub split: f32,
    /// Extra punch-in at full kick, as a fraction.
    pub zoom: f32,
    /// Extra glitch bands this frame.
    pub glitch: usize,
}

/// The frame a Pixel Cats scene draws: the staging into `raster`, then every
/// shared effect into `rgba`, which is the canvas's pixels as RGBA8, row-major
/// from the top, ready to upload.
///
/// Pure: the same state, frame and size give the same bytes.
pub fn render(
    state: &PixelCatsState,
    frame: &SceneFrame<'_>,
    width: i32,
    height: i32,
    raster: &mut Raster,
    rgba: &mut Vec<u8>,
) -> PixelReport {
    let scene = state.id();
    let setting = |index| frame.setting(scene, index);
    let response = setting(control::RESPONSE).max(0.0);
    let motion = setting(control::MOTION).max(0.0);
    let palette = Palette::from_setting(setting(control::PALETTE));
    let shake_amount = setting(control::SHAKE).max(0.0);
    let glitch_amount = setting(control::GLITCH).max(0.0);
    let pace = setting(control::PACE);
    let now = frame.time_seconds;
    let time = now as f32;

    if raster.width() != width || raster.height() != height {
        *raster = Raster::new(width, height);
    }

    // With no response the picture keeps time on its own neutral clock, so the
    // music cannot reach it through the beat either.
    let (phase, beat) = if response > 0.0 {
        (frame.audio.beat_phase.clamp(0.0, 1.0), state.beats)
    } else {
        let clock = (now.max(0.0) * 2.0) as f32;
        (clock.fract(), clock.floor() as u64)
    };
    let (_, _, treble) = frame.audio.spectral_regions();
    let bass = crate::project::caption_effects::EffectInputs::from_audio(
        0.0,
        frame.audio.rms,
        frame.audio.trails,
        0.0,
        0.0,
    )
    .bass;
    let gate = |list: &VecDeque<f64>| {
        if response > 0.0 {
            events(list, now, state.seed)
        } else {
            Vec::new()
        }
    };
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let mut line = frame
        .lyric
        .filter(|lyric| !without_tag(&lyric.text).is_empty())
        .map(|lyric| {
            Line::new(
                lyric.id,
                &lyric.text,
                lyric.start_seconds,
                lyric.end_seconds,
                now,
                pace,
            )
        });
    if let Some(line) = &mut line {
        line.tag = state.current_tag(line.id);
    }
    let moment = Moment {
        staging: state.staging,
        seed: state.seed,
        time,
        clock: time * motion,
        duration: frame.duration_seconds as f32,
        width,
        height,
        phase,
        beat,
        pulse: (-phase * 5.0).exp(),
        kick: finite(state.kick * response),
        vocal: finite(state.vocal * response).min(1.2),
        energy: if response > 0.0 {
            finite(state.energy).min(1.0)
        } else {
            0.5
        },
        bass: finite(bass * response).min(1.2),
        treble: finite(treble * response).min(1.2),
        bands: frame
            .audio
            .bands
            .iter()
            .map(|b| finite(*b * response).clamp(0.0, 1.0))
            .collect(),
        response,
        pops: setting(control::POPS) >= 0.5,
        kicks: gate(&state.kick_times),
        ticks: gate(&state.tick_times),
        onsets: gate(&state.onset_times),
        onset_count: if response > 0.0 { state.onsets_seen } else { 0 },
        travel: state.travel as f32,
        voice_hue: frame
            .lyric
            .and_then(|lyric| lyric.voice.as_ref())
            .and_then(|voice| voice.hue),
        line,
        age: state.age,
        motion,
        history: state.history.iter().cloned().collect(),
        quiet: state.quiet,
        dynamics: state.dynamics,
    };

    let mut report = PixelReport {
        staging: state.staging.name(),
        palette: palette.name(),
        canvas: (width, height, 0),
        beats: state.beats,
        kicks: state.kicks,
        ticks: state.ticks,
        onsets: state.onsets_seen,
        kick: moment.kick,
        energy: moment.energy,
        dynamics: state.dynamics,
        tag: moment.line.as_ref().and_then(|line| line.tag.clone()),
        power: "on",
        ..PixelReport::default()
    };
    report.line = match &moment.line {
        None => "none".into(),
        Some(line) if line.complete() => format!("complete {}", line.total),
        Some(line) => format!("typing {}/{}", line.typed, line.total),
    };

    let ask = stagings::draw(raster, &moment, &mut report);

    // -- the shared hits --------------------------------------------------------
    let tick = (f64::from(time) * 24.0).floor() as u64;
    let key = mix(state.seed ^ tick.wrapping_mul(0x9e37_79b9));
    let amplitude = shake_amount * moment.kick * (1.2 + ask.shake);
    let shake = (
        ((unit(key, 1) * 2.0 - 1.0) * amplitude).round() as i32,
        ((unit(key, 2) * 2.0 - 1.0) * amplitude).round() as i32,
    );
    let split = (shake_amount * (moment.kick * 1.6 + ask.split)).round() as i32;
    let zoom = 1.0 + shake_amount * moment.kick * (0.025 + ask.zoom);
    let mut bands = ask.glitch;
    if state.age < 0.12 {
        bands += 6;
    }
    if moment.line.as_ref().is_some_and(|line| line.elapsed < 0.08) {
        bands += 3;
    }
    // A tear at the end of every fourth bar, on the last eighth of its beat.
    if response > 0.0 && moment.beat % 16 == 15 && moment.phase > 0.78 {
        bands += 4;
    }
    let bands = ((bands as f32) * glitch_amount).round() as usize;
    let mut glitch = Vec::with_capacity(bands);
    for index in 0..bands {
        let salt = 10 + index as u64 * 3;
        let y = (unit(key, salt) * height as f32) as i32;
        let h = 1 + (unit(key, salt + 1) * (height as f32 / 18.0)) as i32;
        let dx = ((unit(key, salt + 2) * 2.0 - 1.0) * width as f32 / 14.0).round() as i32;
        glitch.push((y, y + h, dx));
    }

    // -- the CRT switching off at the very end of the track ----------------------
    let crt = setting(control::CRT);
    let duration = frame.duration_seconds as f32;
    // Fully dark a frame before the end: the last frame an export encodes is
    // a frame before the duration, and it should be the dark one.
    let power = if crt > 0.0 && duration > 2.0 {
        ((time - (duration - POWER_OFF_SECONDS)) / (POWER_OFF_SECONDS - 0.05)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    report.power = if power <= 0.0 {
        "on"
    } else if power >= 1.0 {
        "off"
    } else {
        "switching-off"
    };

    report.shake = shake;
    report.split = split;
    report.zoom = zoom;
    report.glitch_rows = glitch.len();
    let hue = setting(control::HUE);
    compose(
        raster,
        rgba,
        &Compose {
            shake,
            split,
            zoom,
            glitch: &glitch,
            power,
            palette,
            hue,
        },
    );
    report
}

/// How long the picture takes to collapse to a line and a dot before the track
/// ends, when the CRT is on.
pub const POWER_OFF_SECONDS: f32 = 0.7;

struct Compose<'a> {
    shake: (i32, i32),
    split: i32,
    zoom: f32,
    glitch: &'a [(i32, i32, i32)],
    power: f32,
    palette: Palette,
    hue: f32,
}

/// A hue turn as a 3x3 matrix in YIQ space, in 1/1024 fixed point. The same
/// rotation every scene's `hue` control means, done once per frame.
fn hue_matrix(degrees: f32) -> Option<[[i32; 3]; 3]> {
    if !degrees.is_finite() || degrees.abs() < 0.5 {
        return None;
    }
    let (s, c) = degrees.to_radians().sin_cos();
    let to = [
        [0.299f32, 0.587, 0.114],
        [0.596, -0.274, -0.322],
        [0.211, -0.523, 0.312],
    ];
    let from = [
        [1.0f32, 0.956, 0.621],
        [1.0, -0.272, -0.647],
        [1.0, -1.106, 1.703],
    ];
    let rotate = [[1.0f32, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]];
    let multiply = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| {
        let mut out = [[0.0f32; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        out
    };
    let m = multiply(from, multiply(rotate, to));
    let mut fixed = [[0i32; 3]; 3];
    for (i, row) in m.iter().enumerate() {
        for (j, value) in row.iter().enumerate() {
            fixed[i][j] = (value * 1024.0).round() as i32;
        }
    }
    Some(fixed)
}

fn turn(color: Rgb, matrix: &[[i32; 3]; 3]) -> Rgb {
    let v = [i32::from(color.r), i32::from(color.g), i32::from(color.b)];
    let channel = |row: &[i32; 3]| {
        ((row[0] * v[0] + row[1] * v[1] + row[2] * v[2] + 512) >> 10).clamp(0, 255) as u8
    };
    Rgb::new(
        channel(&matrix[0]),
        channel(&matrix[1]),
        channel(&matrix[2]),
    )
}

fn compose(source: &Raster, rgba: &mut Vec<u8>, how: &Compose<'_>) {
    let (w, h) = (source.width(), source.height());
    rgba.resize((w * h * 4) as usize, 255);
    let matrix = hue_matrix(how.hue);
    let ramp = how.palette.ramp().map(|ramp| match &matrix {
        Some(m) => ramp.map(|c| turn(c, m)),
        None => ramp,
    });
    let map = |c: Rgb| -> Rgb {
        match (&ramp, &matrix) {
            (Some(ramp), _) => ramp[(usize::from(c.luma()) * 4 / 256).min(3)],
            (None, Some(m)) => turn(c, m),
            (None, None) => c,
        }
    };
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let inverse = 1.0 / how.zoom.max(1.0);
    // The power-off: the picture squashes to a bright bar, the bar to a dot.
    let squash = (how.power / 0.6).clamp(0.0, 1.0);
    let visible = ((h as f32) * (1.0 - squash)).max(2.0);
    let shrink = ((how.power - 0.6) / 0.4).clamp(0.0, 1.0);
    for y in 0..h {
        let offset = how
            .glitch
            .iter()
            .filter(|&&(a, b, _)| y >= a && y < b)
            .map(|&(_, _, dx)| dx)
            .sum::<i32>();
        let row = (y * w * 4) as usize;
        for x in 0..w {
            let out = row + (x * 4) as usize;
            if how.power > 0.0 {
                let top = (h as f32 - visible) * 0.5;
                let inside_rows = (y as f32) >= top && (y as f32) < top + visible;
                let bar = (w as f32) * (1.0 - shrink);
                let inside_bar = (x as f32 - cx).abs() <= bar * 0.5 + 0.5;
                if how.power >= 1.0 || !inside_rows || (shrink > 0.0 && !inside_bar) {
                    rgba[out..out + 4].copy_from_slice(&[0, 0, 0, 255]);
                    continue;
                }
                if shrink > 0.0 {
                    rgba[out..out + 4].copy_from_slice(&[255, 255, 255, 255]);
                    continue;
                }
            }
            let mut sx =
                (cx + (x as f32 + 0.5 - cx) * inverse).floor() as i32 - how.shake.0 - offset;
            let mut sy = (cy + (y as f32 + 0.5 - cy) * inverse).floor() as i32 - how.shake.1;
            if how.power > 0.0 {
                // Rows are resampled into the shrinking band, brightened.
                let top = (h as f32 - visible) * 0.5;
                sy = (((y as f32 - top) / visible) * h as f32) as i32;
                sx = x;
            }
            let base = source.get(sx, sy);
            let color = if how.split != 0 {
                Rgb::new(
                    source.get(sx + how.split, sy).r,
                    base.g,
                    source.get(sx - how.split, sy).b,
                )
            } else {
                base
            };
            let mut color = map(color);
            if how.power > 0.0 {
                color = color
                    .scale(1.0 + 2.0 * squash)
                    .mix(Rgb::WHITE, 0.25 * squash);
            }
            rgba[out] = color.r;
            rgba[out + 1] = color.g;
            rgba[out + 2] = color.b;
            rgba[out + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests;
