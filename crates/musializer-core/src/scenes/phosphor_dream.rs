//! Phosphor Dream: deterministic state, the field bank, and grid evaluation.
//!
//! **No oracle.** This is the first scene in the tree with no `scene_*.c` behind
//! it — a post-legacy addition (2026-08-08) under the rule in `AGENTS.md` that
//! features past the frozen C's ceiling need no parity justification. There is
//! nothing to cite and nothing to diff against, so the evidence for this file is
//! its own tests.
//!
//! ## What it is
//!
//! A generative ASCII screensaver: a grid of glyphs driven by ten procedural
//! scalar fields (plasma, tunnel, kaleidoscope, metaballs, rain, hyperspace,
//! vortex, moiré, ripples, domain-warped noise), cycling on a dwell clock with a
//! dithered crossfade between alphabets, under a global slow rotate/zoom/ripple
//! of the whole coordinate space. The drawing half adds the CRT: bloom, colour
//! split, scanlines and a rolling refresh band.
//!
//! Adapted with permission from a Python offline renderer by **Digi**
//! (<https://github.com/digi-the-robot>, <https://x.com/digi_dot_exe>,
//! canonical: <https://linktr.ee/digi_the_robot>) — the source is
//! `OUTSIDE-DROPS/dreamscape.py` plus its `README.md`.
//!
//! ## What changed on the way in, and why
//!
//! The source renders offline at a fixed 160x54 into numpy arrays, then pushes
//! whole frames through a CPU post chain. None of that survives contact with a
//! realtime scene, and the differences are deliberate:
//!
//! - **The grid is not fixed.** 160x54 min-fitted into a 9:16 export would draw a
//!   band across a quarter of the frame — the same defect EX2 fixed in ASCII
//!   Field. The cell edge derives from the frame's short axis and the counts fill
//!   it, so a portrait render is a portrait field.
//! - **Audio comes from the analyzer, not from an FFT of the file.** The source
//!   decodes the track up front and precomputes per-frame envelopes; a live scene
//!   has [`SceneAudioFrame`] already. The fast-attack/slow-release follower is
//!   kept, because that shape is what makes the coupling read as breathing rather
//!   than jitter, but it runs per frame in [`PhosphorDreamState::update`] and is
//!   expressed as a **per-second** retention so preview and export agree at any
//!   frame rate. The source's `0.86` is per frame at 30 fps, which would pump
//!   twice as fast in a 60 fps preview as in the file it rendered.
//! - **The alphabets are half shapes.** raylib's built-in face — the one ASCII
//!   Field draws through, and the only monospaced face in the tree — covers
//!   Latin-1 and stops. The block elements, geometric shapes and stars the source
//!   leans on are all above U+2500. Bundling a monospace TTF with that coverage
//!   is a new third-party asset and licence for ~20 glyphs, so [`Ink::Shape`]
//!   synthesizes them instead. That also removes a magnified-bitmap blur the
//!   caption work already had to answer for once: a shape drawn from geometry is
//!   correct at 12 px and at 200 px under export supersampling.
//! - **The words that surface are the track's lyrics.** The source hardcodes six
//!   of its own. This application has authored cue timing and a document behind
//!   it, so the `titles` toggle surfaces [`SceneFrame::lyric`] instead. Nothing
//!   surfaces when there is no cue, rather than a canned word appearing over
//!   somebody's track.
//!
//! Everything here is pure: no clock, no I/O, and the only randomness is derived
//! from the instance seed.

use std::any::Any;

use crate::project::caption_effects::bass_from_trails;
use crate::scene::settings::index::phosphor as setting;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use crate::ui::tune_explore::{RandomSource, SplitMix64};

/// Bumped when the state layout changes so a rebind discards stale state.
pub const STATE_VERSION: u32 = 1;

const TAU: f32 = std::f32::consts::TAU;

/// How many procedural fields the bank holds.
///
/// Equal to the upper bound of the `field` setting, and the two are asserted
/// equal in the tests: the setting says "0 = cycle, 1..=10 pins one", so a bank
/// that grew without the descriptor growing would make the last field
/// unreachable from the inspector.
pub const FIELD_COUNT: usize = 10;

/// How many character alphabets the bank holds. Same relationship to the `ramp`
/// setting's upper bound.
pub const RAMP_COUNT: usize = 7;

/// Seconds of crossfade between fields, before the dwell clamp below.
///
/// The source's value. It is long on purpose — the dissolve is dithered between
/// two alphabets rather than blended, and a short one reads as a glitch.
pub const FADE_SECONDS: f32 = 2.6;

/// The fraction of a dwell the crossfade may occupy.
///
/// `dwell` goes down to 4 s, where a flat 2.6 s fade would leave 1.4 s of settled
/// picture and the scene would read as permanently dissolving. Clamping to a
/// proportion keeps the *feel* of the fade at every dwell.
const FADE_DWELL_FRACTION: f32 = 0.26;

/// Per-second retention of the envelope follower's release limb.
///
/// The source uses `acc = acc * 0.86 + s * 0.14` once per frame at 30 fps.
/// `0.86^30` is this number, and raising it to `delta_seconds` reproduces the
/// same decay at any frame rate — which matters here because a preview runs at
/// 60 and an export at whatever the project asks for, and a coupling that pumped
/// at different speeds in the two would break the stated preview/export
/// invariant.
const ENVELOPE_RELEASE_PER_SECOND: f32 = 0.010_69;

// -- the glyph alphabet --------------------------------------------------------

/// A shape the drawing half synthesizes, because raylib's built-in face has no
/// glyph for it.
///
/// These stand in for the source's block elements, geometric shapes and stars
/// (U+2591..U+2588, U+25A0..U+25C7, U+2248/U+2261, U+203B/U+2726). Drawing them
/// from geometry rather than from a bundled face is what keeps them sharp under
/// export supersampling, where a bitmap atlas glyph magnified past its native
/// size is the blur the caption work already had to fix once.
///
/// The variants carry *what to draw*, not how — the drawing half owns cell size,
/// stroke weight and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// A solid cell at `quarters/4` coverage — the shade blocks `░▒▓█`.
    ///
    /// The source's shade blocks are dither patterns; at the sizes this scene
    /// draws at, a flat rectangle at matching coverage is visually the same and
    /// does not alias into moiré against the scanlines.
    Fill(u8),
    /// `●` — a filled disc at most of the cell.
    Disc,
    /// `◦` / `∘` — a small ring.
    RingSmall,
    /// `○` — a large ring.
    RingLarge,
    /// `◆` / `◇` — a diamond, filled or outlined.
    Diamond { filled: bool },
    /// `▪`/`▫` (small) and `■`/`□` (large) — a square, filled or outlined.
    Square { filled: bool, large: bool },
    /// `▩` / `▨` — diagonal hatching, one direction each.
    Hatch { back: bool },
    /// `≈` (two) and `≡` (three) — stacked horizontal bars.
    Bars(u8),
    /// `✦` — a four-point star.
    Star4,
    /// `※` — a six-spoke asterisk.
    Star6,
}

/// One entry in an alphabet.
///
/// Split three ways rather than carrying a `char` throughout because the third
/// case genuinely cannot be drawn as text here, and an `Ink::Char('█')` that the
/// font silently renders as an empty box is exactly the class of failure this
/// repository keeps paying for — a plausible picture with a hole in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ink {
    /// Draw nothing. The dark end of every ramp.
    Blank,
    /// A codepoint raylib's built-in face has: U+0020..=U+00FF.
    Char(char),
    /// A synthesized shape.
    Shape(Shape),
}

impl Ink {
    /// Whether this entry puts anything on the cell at all.
    #[must_use]
    pub fn is_blank(self) -> bool {
        matches!(self, Ink::Blank)
    }

    /// The codepoint, for the entries that have one.
    ///
    /// Used by the drawing half to pick the text path, and by the test that
    /// asserts every `Char` is inside the built-in face's coverage.
    #[must_use]
    pub fn codepoint(self) -> Option<u32> {
        match self {
            Ink::Char(character) => Some(character as u32),
            _ => None,
        }
    }
}

const fn c(character: char) -> Ink {
    Ink::Char(character)
}

const fn s(shape: Shape) -> Ink {
    Ink::Shape(shape)
}

/// The seven alphabets, dark to bright.
///
/// Order inside a ramp is load-bearing: a cell's value indexes it directly, so
/// an entry out of luminance order makes the field look like it has holes in it.
/// The names are the source's.
#[rustfmt::skip]
pub const RAMPS: [&[Ink]; RAMP_COUNT] = [
    // "soft" — the classic dark-to-light ASCII ramp.
    &[Ink::Blank, c('.'), c('`'), c('\''), c(','), c(':'), c(';'), c('~'), c('-'),
      c('+'), c('='), c('*'), c('o'), c('O'), c('0'), c('#'), c('%'), c('@')],
    // "blocks" — the shade ladder. Two blanks and three dots at the bottom so the
    // dark end stays open rather than jumping straight to a quarter-filled cell.
    &[Ink::Blank, Ink::Blank, c('.'), c('.'), c('.'), c(':'), c(':'), c(':'),
      s(Shape::Fill(1)), s(Shape::Fill(1)), s(Shape::Fill(2)), s(Shape::Fill(2)),
      s(Shape::Fill(3)), s(Shape::Fill(3)), s(Shape::Fill(4)), s(Shape::Fill(4))],
    // "tech" — punctuation-heavy, the densest-reading of the Latin-1 ramps.
    &[Ink::Blank, c('.'), c(':'), c('-'), c('='), c('+'), c('*'), c('#'),
      c('%'), c('@'), c('$'), c('&'), c('8'), c('B'), c('@')],
    // "matrix" — the falling-glyph alphabet. `Æ` is Latin-1, so it draws as text.
    &[Ink::Blank, c('.'), c(':'), c(';'), c('+'), c('='), c('x'), c('X'),
      c('Æ'), c('#'), c('$'), c('@'), c('1'), c('0')],
    // "stars" — sparse, for the hyperspace field. `×` is Latin-1; the last two
    // are shapes.
    &[Ink::Blank, Ink::Blank, c('.'), c('.'), c('`'), c('`'), c('\''), c('\''),
      c('+'), c('*'), c('x'), c('×'), s(Shape::Star6), s(Shape::Star4), s(Shape::Disc)],
    // "wave" — for the interference fields.
    &[Ink::Blank, c('~'), c('-'), c('_'), c('='), s(Shape::Bars(2)), s(Shape::Bars(3)),
      c('+'), c('*'), c('#'), c('%'), c('@')],
    // "glyph" — geometric, almost all synthesized. `·` is Latin-1.
    &[Ink::Blank, c('.'), c('·'), c(':'), s(Shape::RingSmall), s(Shape::RingLarge),
      s(Shape::RingSmall), s(Shape::Disc), s(Shape::Diamond { filled: true }),
      s(Shape::Diamond { filled: false }), s(Shape::Square { filled: true, large: false }),
      s(Shape::Square { filled: false, large: false }),
      s(Shape::Square { filled: true, large: true }),
      s(Shape::Square { filled: false, large: true }),
      s(Shape::Hatch { back: false }), s(Shape::Hatch { back: true })],
];

/// The alphabet names, for the inspector readout and the report line.
pub const RAMP_NAMES: [&str; RAMP_COUNT] =
    ["soft", "blocks", "tech", "matrix", "stars", "wave", "glyph"];

// -- the field bank ------------------------------------------------------------

/// One procedural scalar field.
///
/// Each maps a warped coordinate and a time to `(value, hue)`, both nominally
/// `0..1` — value is brightness/density and picks the glyph, hue is a position on
/// the colour wheel. Two of them ([`Field::Rain`] and [`Field::Stars`]) are not
/// pure functions of `(x, y)`: they read the grid's own row/column, which is why
/// evaluation goes through [`evaluate_grid`] rather than a bare `fn(f32, f32)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// Domain-warped noise: plasma fed back through itself.
    Warpnoise,
    /// The demoscene four-sine field.
    Plasma,
    /// Infinite zoom down a checkered throat.
    Tunnel,
    /// Six-fold mirrored wedge, slowly rotating.
    Kaleido,
    /// Seven metaballs orbiting each other.
    Lava,
    /// Falling glyph columns.
    Rain,
    /// Hyperspace: stars accelerating outward.
    Stars,
    /// Logarithmic spiral.
    Vortex,
    /// Two rotating grids beating against each other.
    Moire,
    /// Four wandering wave sources interfering.
    Ripple,
}

impl Field {
    /// The bank in its default order, which is the order pass 0 of the cycle
    /// runs them in.
    pub const ALL: [Field; FIELD_COUNT] = [
        Field::Warpnoise,
        Field::Plasma,
        Field::Tunnel,
        Field::Kaleido,
        Field::Lava,
        Field::Rain,
        Field::Stars,
        Field::Vortex,
        Field::Moire,
        Field::Ripple,
    ];

    /// The name the inspector and the report line use.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Field::Warpnoise => "warpnoise",
            Field::Plasma => "plasma",
            Field::Tunnel => "tunnel",
            Field::Kaleido => "kaleido",
            Field::Lava => "lava",
            Field::Rain => "rain",
            Field::Stars => "stars",
            Field::Vortex => "vortex",
            Field::Moire => "moire",
            Field::Ripple => "ripple",
        }
    }

    /// The alphabet this field looks best in, used when `ramp` is 0 (auto).
    ///
    /// These pairings are the source's and they matter more than any other single
    /// choice in the file: the alphabet changes a field's texture more than its
    /// own parameters do.
    #[must_use]
    pub fn default_ramp(self) -> usize {
        match self {
            Field::Warpnoise => 0,              // soft
            Field::Plasma | Field::Lava => 1,   // blocks
            Field::Tunnel | Field::Moire => 2,  // tech
            Field::Rain => 3,                   // matrix
            Field::Stars => 4,                  // stars
            Field::Vortex | Field::Ripple => 5, // wave
            Field::Kaleido => 6,                // glyph
        }
    }

    /// Whether this field splats into the grid rather than reading each cell
    /// independently.
    #[must_use]
    fn is_splatting(self) -> bool {
        matches!(self, Field::Stars)
    }
}

// -- deterministic state -------------------------------------------------------

/// Per-column falling-trail parameters, indexed by `column % RAIN_TABLE`.
///
/// A fixed table rather than one entry per drawn column, because the drawn
/// column count changes with the frame's aspect and window size, and a table
/// that resized would make the rain restart every time the user dragged the
/// window edge.
const RAIN_TABLE: usize = 256;

/// How many stars the hyperspace field carries. The source's count.
const STAR_COUNT: usize = 1100;

/// The deterministic half of Phosphor Dream.
///
/// Three things live here and nothing else does: the audio envelope followers,
/// the field-cycle clock, and the seed-derived tables the rain and star fields
/// read. Everything else about a frame is a pure function of these plus the
/// frame, which is what lets a seek land on the same picture twice.
#[derive(Clone, Debug)]
pub struct PhosphorDreamState {
    seed: u64,
    /// Overall loudness, fast attack and slow release, `0..1`.
    amplitude: f32,
    /// Low-band energy on the same follower — kick and bassline.
    bass: f32,
    /// Seconds accumulated on the cycle clock.
    ///
    /// Integrated from `delta_seconds` rather than read from `time_seconds`
    /// because `dwell` is a live setting: dividing the absolute track time by a
    /// dwell the user just moved would jump the cycle to a different field
    /// mid-drag. Integrating means changing the dwell changes what happens
    /// *next*.
    cycle_seconds: f32,
    /// Which slot of the cycle is current. Grows without bound; the pass number
    /// is `slot / FIELD_COUNT`.
    slot: usize,
    /// Seconds the current slot has been showing.
    slot_seconds: f32,
    rain_speed: [f32; RAIN_TABLE],
    rain_offset: [f32; RAIN_TABLE],
    rain_length: [f32; RAIN_TABLE],
    star_angle: Box<[f32; STAR_COUNT]>,
    star_depth: Box<[f32; STAR_COUNT]>,
    star_speed: Box<[f32; STAR_COUNT]>,
    star_hue: Box<[f32; STAR_COUNT]>,
}

impl PhosphorDreamState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed ^ 0x0DEA_D0DE_CAFE_F00D);
        let mut rain_speed = [0.0f32; RAIN_TABLE];
        let mut rain_offset = [0.0f32; RAIN_TABLE];
        let mut rain_length = [0.0f32; RAIN_TABLE];
        for index in 0..RAIN_TABLE {
            rain_speed[index] = rng.next_range(6.0, 22.0) as f32;
            rain_offset[index] = rng.next_range(0.0, 200.0) as f32;
            rain_length[index] = rng.next_range(8.0, 26.0) as f32;
        }
        let mut star_angle = Box::new([0.0f32; STAR_COUNT]);
        let mut star_depth = Box::new([0.0f32; STAR_COUNT]);
        let mut star_speed = Box::new([0.0f32; STAR_COUNT]);
        let mut star_hue = Box::new([0.0f32; STAR_COUNT]);
        for index in 0..STAR_COUNT {
            star_angle[index] = rng.next_range(0.0, f64::from(TAU)) as f32;
            star_depth[index] = rng.next_unit() as f32;
            star_speed[index] = rng.next_range(0.10, 0.30) as f32;
            star_hue[index] = rng.next_unit() as f32;
        }
        Self {
            seed,
            amplitude: 0.0,
            bass: 0.0,
            cycle_seconds: 0.0,
            slot: 0,
            slot_seconds: 0.0,
            rain_speed,
            rain_offset,
            rain_length,
            star_angle,
            star_depth,
            star_speed,
            star_hue,
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn amplitude(&self) -> f32 {
        self.amplitude
    }

    #[must_use]
    pub fn bass(&self) -> f32 {
        self.bass
    }

    #[must_use]
    pub fn slot(&self) -> usize {
        self.slot
    }

    #[must_use]
    pub fn slot_seconds(&self) -> f32 {
        self.slot_seconds
    }

    /// The field and alphabet a cycle slot resolves to.
    ///
    /// Pass 0 runs [`Field::ALL`] in order with each field's own alphabet. Every
    /// later pass reshuffles the order and rotates the alphabets one step, so a
    /// long track is *more material* rather than the same loop — the source's
    /// argument, and the reason a forty-minute set does not become recognisably
    /// periodic after ten minutes.
    #[must_use]
    pub fn slot_program(&self, slot: usize) -> (Field, usize) {
        let pass = slot / FIELD_COUNT;
        let position = slot % FIELD_COUNT;
        if pass == 0 {
            let field = Field::ALL[position];
            return (field, field.default_ramp());
        }
        // A fresh permutation per pass, derived from the instance seed so two
        // instances of the scene do not march in step.
        let mut order = Field::ALL;
        let mut rng = SplitMix64::new(
            self.seed
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(pass as u64),
        );
        for index in (1..FIELD_COUNT).rev() {
            let swap = (rng.next_u64() % (index as u64 + 1)) as usize;
            order.swap(index, swap);
        }
        let field = order[position];
        let ramp = (field.default_ramp() + pass) % RAMP_COUNT;
        (field, ramp)
    }

    /// Seconds of crossfade for a given dwell.
    #[must_use]
    pub fn fade_seconds(dwell: f32) -> f32 {
        FADE_SECONDS.min(dwell * FADE_DWELL_FRACTION)
    }

    /// The low-band energy the bass follower is fed, `0..1`.
    ///
    /// Delegated to [`bass_from_trails`] rather than derived here, because this
    /// application already had a definition of "bass" — the one the caption glow
    /// is driven by — and a scene with its own would be a second number under
    /// the same name. The first version of this function did exactly that: it
    /// averaged the lowest eighth of the *instantaneous* bands, and reported
    /// `bass=0.00` on every frame of the synthetic fixture while the caption
    /// drive on those same frames was working.
    #[must_use]
    pub fn raw_bass(audio: &SceneAudioFrame<'_>) -> f32 {
        clamp01(bass_from_trails(audio.trails))
    }

    /// The overall loudness the amplitude follower is fed, `0..1`.
    ///
    /// The same `rms * 2` normalization ASCII Field uses for its energy term, so
    /// the two CRT scenes respond to a track at comparable strength.
    #[must_use]
    pub fn raw_amplitude(audio: &SceneAudioFrame<'_>) -> f32 {
        clamp01(audio.rms * 2.0)
    }

    /// One step of the fast-attack, slow-release follower.
    ///
    /// Instant on the way up, exponential on the way down. That asymmetry is the
    /// whole point: a symmetric filter either lags the transient or chatters, and
    /// what makes a visual read as *pumping with* a track is catching the hit and
    /// letting go slowly.
    #[must_use]
    pub fn follow(previous: f32, target: f32, delta_seconds: f32) -> f32 {
        if !target.is_finite() {
            return previous;
        }
        if target >= previous {
            return target;
        }
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return previous;
        }
        let retain = ENVELOPE_RELEASE_PER_SECOND.powf(delta_seconds.min(1.0));
        target + (previous - target) * retain
    }
}

impl SceneState for PhosphorDreamState {
    fn id(&self) -> SceneId {
        SceneId::PhosphorDream
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        let delta = if frame.delta_seconds.is_finite() && frame.delta_seconds > 0.0 {
            // A long stall — a seek, a dropped frame, a debugger — must not
            // advance the cycle by a whole field. Capped rather than ignored,
            // because ignoring it would freeze the clock during a slow export.
            frame.delta_seconds.min(0.25)
        } else {
            0.0
        };

        self.amplitude = Self::follow(
            self.amplitude,
            Self::raw_amplitude(&frame.audio),
            frame.delta_seconds,
        );
        self.bass = Self::follow(self.bass, Self::raw_bass(&frame.audio), frame.delta_seconds);

        let dwell = frame
            .setting(SceneId::PhosphorDream, setting::DWELL)
            .max(1.0);
        // A pinned field stops the clock rather than letting it run invisibly.
        // Otherwise unpinning would jump to wherever the cycle had wandered to,
        // which reads as the control being broken.
        let pinned = frame.setting(SceneId::PhosphorDream, setting::FIELD) >= 0.5;
        if pinned {
            self.slot_seconds = 0.0;
            return;
        }
        self.cycle_seconds += delta;
        self.slot_seconds += delta;
        while self.slot_seconds >= dwell {
            self.slot_seconds -= dwell;
            self.slot = self.slot.wrapping_add(1);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The registry entry.
#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::PhosphorDream,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(PhosphorDreamState::new(seed)),
    }
}

// -- resolved per-frame parameters ---------------------------------------------

/// The scene's settings resolved for one frame, plus the audio the fields read.
///
/// Built once per frame by the drawing half and handed to [`evaluate_grid`]. A
/// struct rather than a dozen arguments because the grid evaluation is the one
/// place preview and export must agree exactly, and a positional argument list
/// that long is where they would eventually stop agreeing.
#[derive(Clone, Copy, Debug)]
pub struct FieldParams {
    /// Seconds on the field clock. Wrapped by the caller so the sine arguments
    /// keep float resolution over a long set.
    pub time: f32,
    pub amplitude: f32,
    pub bass: f32,
    /// The current field and its alphabet.
    pub field: Field,
    pub ramp: usize,
    /// The outgoing field during a crossfade, and how far through it is (`0..1`,
    /// smoothstepped). `None` outside a fade.
    pub previous: Option<(Field, usize)>,
    pub mix: f32,
    /// `settings.phosphor.motion`.
    pub motion: f32,
    /// `settings.phosphor.breathe`.
    pub breathe: f32,
    /// `settings.phosphor.hue`, in turns rather than degrees.
    pub hue_shift: f32,
    /// `settings.phosphor.reactivity`.
    pub reactivity: f32,
    /// Cell aspect — cell width over cell height — so the coordinate space is
    /// square even though the cells are not.
    pub cell_aspect: f32,
    /// The instance seed, for the dither pattern and the rain/star tables.
    pub seed: u64,
}

impl FieldParams {
    /// Everything except the geometry, resolved from a frame and a state.
    ///
    /// Kept here rather than in the drawing half so the resolution — including
    /// which slot is showing and whether a fade is running — is testable without
    /// a window.
    #[must_use]
    pub fn resolve(state: &PhosphorDreamState, frame: &SceneFrame<'_>, cell_aspect: f32) -> Self {
        let scene = SceneId::PhosphorDream;
        let dwell = frame.setting(scene, setting::DWELL).max(1.0);
        let pin = frame.setting(scene, setting::FIELD);
        let ramp_pin = frame.setting(scene, setting::RAMP);

        let (field, auto_ramp, previous, mix) = if pin >= 0.5 {
            // Pinned: no cycle, no fade.
            let index = ((pin.round() as i32 - 1).max(0) as usize).min(FIELD_COUNT - 1);
            let field = Field::ALL[index];
            (field, field.default_ramp(), None, 0.0)
        } else {
            let (field, ramp) = state.slot_program(state.slot);
            let fade = PhosphorDreamState::fade_seconds(dwell);
            let previous = if state.slot > 0 && state.slot_seconds < fade && fade > 0.0 {
                Some(state.slot_program(state.slot - 1))
            } else {
                None
            };
            let mix = match previous {
                // Smoothstep, so the dissolve eases at both ends instead of
                // starting and stopping abruptly.
                Some(_) => {
                    let u = clamp01(state.slot_seconds / fade);
                    u * u * (3.0 - 2.0 * u)
                }
                None => 0.0,
            };
            (field, ramp, previous, mix)
        };

        let resolve_ramp = |auto: usize| -> usize {
            if ramp_pin >= 0.5 {
                ((ramp_pin.round() as i32 - 1).max(0) as usize).min(RAMP_COUNT - 1)
            } else {
                auto
            }
        };

        let time = if frame.time_seconds.is_finite() {
            // Wrapped at 4096 s for the same reason ASCII Field wraps: past that
            // a float's sine argument loses the resolution the animation needs.
            (frame.time_seconds % 4096.0) as f32
        } else {
            0.0
        };

        Self {
            time,
            amplitude: state.amplitude(),
            bass: state.bass(),
            field,
            ramp: resolve_ramp(auto_ramp),
            previous: previous.map(|(f, r)| (f, resolve_ramp(r))),
            mix,
            motion: frame.setting(scene, setting::MOTION),
            breathe: frame.setting(scene, setting::BREATHE),
            hue_shift: frame.setting(scene, setting::HUE) / 360.0,
            reactivity: frame.setting(scene, setting::REACTIVITY),
            cell_aspect: if cell_aspect.is_finite() && cell_aspect > 0.0 {
                cell_aspect
            } else {
                1.0
            },
            seed: state.seed(),
        }
    }

    /// The alphabet currently in front.
    #[must_use]
    pub fn ramp_name(&self) -> &'static str {
        RAMP_NAMES[self.ramp.min(RAMP_COUNT - 1)]
    }
}

// -- grid evaluation -----------------------------------------------------------

/// One evaluated cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridCell {
    /// Brightness/density after shaping, `0..1`.
    pub value: f32,
    /// Colour wheel position, `0..1`.
    pub hue: f32,
    /// Saturation, `0..1`.
    pub saturation: f32,
    /// The glyph to put on the cell.
    pub ink: Ink,
}

impl Default for GridCell {
    fn default() -> Self {
        Self {
            value: 0.0,
            hue: 0.0,
            saturation: 1.0,
            ink: Ink::Blank,
        }
    }
}

/// An evaluated frame of the glyph field.
///
/// Owned by the drawing half and reused across frames — allocating an 8,000-cell
/// vector every frame is the kind of thing that is invisible in a preview and
/// shows up as a stutter in a long export.
#[derive(Clone, Debug, Default)]
pub struct FieldGrid {
    columns: usize,
    rows: usize,
    cells: Vec<GridCell>,
    /// Scratch for the two layers a crossfade blends, and for the star splat.
    front: Vec<(f32, f32)>,
    back: Vec<(f32, f32)>,
    splat: Vec<(f32, f32)>,
}

impl FieldGrid {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub fn cell(&self, row: usize, column: usize) -> Option<&GridCell> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        self.cells.get(row * self.columns + column)
    }

    fn resize(&mut self, columns: usize, rows: usize) {
        let count = columns * rows;
        if self.columns != columns || self.rows != rows {
            self.columns = columns;
            self.rows = rows;
            self.cells.clear();
            self.cells.resize(count, GridCell::default());
            self.front.clear();
            self.front.resize(count, (0.0, 0.0));
            self.back.clear();
            self.back.resize(count, (0.0, 0.0));
            self.splat.clear();
            self.splat.resize(count, (0.0, 0.0));
        }
    }
}

/// Clamps to `0..1`, NaN to zero.
#[must_use]
pub fn clamp01(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// The global slow rotate / zoom / ripple of the whole coordinate space.
///
/// Nothing in this scene ever sits still, and this is why. The bass term punches
/// the camera in on a kick — the source's ~5 %, scaled by `reactivity` here so
/// the coupling is authorable like every other drive in the application.
fn breathe(params: &FieldParams, base_x: f32, base_y: f32) -> (f32, f32) {
    let t = params.time;
    let strength = params.breathe;
    let rotation = (0.28 * (t * 0.083).sin() + 0.11 * (t * 0.031).sin()) * strength;
    let zoom = 1.0 + (0.30 * (t * 0.137).sin() + 0.12 * (t * 0.061 + 1.3).sin()) * strength
        - 0.055 * params.bass * params.reactivity;
    let (sin_r, cos_r) = rotation.sin_cos();
    let x = (base_x * cos_r - base_y * sin_r) * zoom;
    let y = (base_x * sin_r + base_y * cos_r) * zoom;
    let ripple = 0.14 * strength;
    (
        x + ripple * (y * 2.3 + t * 0.55).sin(),
        y + ripple * (x * 2.1 - t * 0.47).sin(),
    )
}

/// Evaluates one field at one warped coordinate.
///
/// `row`/`column` are passed for the two fields that read the grid directly; the
/// other eight ignore them.
#[allow(clippy::too_many_arguments)]
fn evaluate_field(
    field: Field,
    state: &PhosphorDreamState,
    params: &FieldParams,
    x: f32,
    y: f32,
    row: usize,
    rows: usize,
    column: usize,
) -> (f32, f32) {
    let t = params.time * params.motion;
    match field {
        Field::Plasma => {
            let r = (x * x + y * y).sqrt();
            let v = ((x * 3.1 + t * 0.7).sin()
                + (y * 2.6 - t * 0.53).sin()
                + ((x + y) * 2.2 + t * 0.91).sin()
                + (r * 4.4 - t * 1.27).sin())
                * 0.25;
            let value = 0.5 + 0.5 * v;
            (value, 0.62 + value * 0.45 + t * 0.035)
        }
        Field::Warpnoise => {
            let q1 = (x * 1.9 + t * 0.31).sin();
            let q2 = (y * 1.7 - t * 0.27).cos();
            let p1 = ((x + q1 * 1.2) * 2.7 + t * 0.44).sin();
            let p2 = ((y + q2 * 1.2) * 2.5 - t * 0.39).cos();
            let mut w = ((x + p1 * 1.6) * 2.0 + (y + p2 * 1.6) * 2.0 + t * 0.62).sin();
            w += 0.6 * ((x - p2) * 4.5 - (y + p1) * 4.1 - t * 0.8).sin();
            (
                clamp01(0.5 + 0.34 * w),
                0.05 + 0.5 * p1 + 0.3 * p2 + t * 0.045,
            )
        }
        Field::Tunnel => {
            let r = (x * x + y * y).sqrt() + 1e-3;
            let a = y.atan2(x);
            let d = 1.0 / r;
            let u = d * 1.15 + t * 0.85;
            let w = a / TAU * 8.0 + 0.35 * (d * 0.8 + t * 0.5).sin();
            let mut value = 0.5 + 0.5 * (u * TAU).sin() * (w * TAU).sin();
            value *= clamp01(r * 2.4) * clamp01(2.2 - r);
            (clamp01(value), u * 0.11 + w * 0.03 + t * 0.04)
        }
        Field::Kaleido => {
            let a0 = y.atan2(x) + t * 0.13;
            let r = (x * x + y * y).sqrt();
            let folds = 6.0;
            let wedge = TAU / folds;
            let a = (a0.rem_euclid(wedge) - wedge * 0.5).abs() * folds;
            let (xx, yy) = (r * a.cos(), r * a.sin());
            let v = ((xx * 4.0 + t * 0.8).sin()
                + (yy * 3.4 - t * 0.6).sin()
                + (r * 6.0 - t * 1.1).sin())
                / 3.0;
            let value = clamp01(0.5 + 0.55 * v) * clamp01(2.0 - r * 0.9);
            (value, 0.3 + r * 0.22 + t * 0.05 + v * 0.15)
        }
        Field::Lava => {
            let mut sum = 0.0f32;
            let mut hue_accumulator = 0.0f32;
            for index in 0..7 {
                // The source seeds seven fixed phase quadruples from a numpy RNG.
                // Derived from the instance seed here instead, so two Phosphor
                // Dreams in one project do not have identical lamps.
                let phase = blob_phase(params.seed, index);
                let bx = 1.35 * (t * 0.23 + phase.0).sin() * (t * 0.11 + phase.1).cos();
                let by = 0.80 * (t * 0.19 + phase.2).sin();
                let radius = 0.16 + 0.09 * (t * 0.4 + phase.3).sin();
                let d2 = (x - bx) * (x - bx) + (y - by) * (y - by) + 1e-3;
                let g = radius / d2;
                sum += g;
                hue_accumulator += g * (index as f32 / 7.0);
            }
            let hue = 0.02 + hue_accumulator / (sum + 1e-3) * 0.8 + t * 0.03;
            (clamp01(sum * 0.30).powf(0.75), hue)
        }
        Field::Rain => {
            let table = column % RAIN_TABLE;
            let speed = state.rain_speed[table];
            let offset = state.rain_offset[table];
            let length = state.rain_length[table];
            let span = rows as f32 + 40.0;
            let head = (t * speed + offset).rem_euclid(span) - 20.0;
            let distance = head - row as f32;
            let mut value = if distance >= 0.0 {
                (-distance / length).exp() * 1.35
            } else {
                0.0
            };
            // The hot head of the trail, brighter than the tail behind it.
            value += 0.75 * (-distance.abs() * 1.1).exp();
            let flicker = 0.80 + 0.20 * (row as f32 * 12.7 + column as f32 * 4.3 + t * 9.0).sin();
            let value = clamp01(value * flicker) + 0.05;
            (
                value,
                0.33 + 0.06 * (column as f32 * 0.2 + t * 0.3).sin() + 0.10 * (1.0 - value),
            )
        }
        Field::Stars => {
            // Handled by the splat pre-pass; this arm is unreachable through
            // `evaluate_grid` and returns a dead field rather than panicking, so a
            // future caller that forgets the pre-pass gets a dark frame and the
            // report line rather than a crash mid-export.
            let _ = (row, column);
            (0.0, 0.0)
        }
        Field::Vortex => {
            let r = (x * x + y * y).sqrt() + 1e-3;
            let a = y.atan2(x);
            let spiral = a * 3.0 + r.ln() * 5.0 - t * 1.5;
            let mut value = 0.5 + 0.5 * spiral.sin();
            value *= clamp01(r * 3.0);
            (
                clamp01(value).powf(1.3),
                0.75 + (spiral * 0.5).sin() * 0.18 + t * 0.04 + r * 0.1,
            )
        }
        Field::Moire => {
            let g1 = (x * 9.0 + t * 0.4).sin() * (y * 9.0 - t * 0.3).sin();
            let angle = t * 0.11;
            let (sin_a, cos_a) = angle.sin_cos();
            let xr = x * cos_a - y * sin_a;
            let yr = x * sin_a + y * cos_a;
            let scale = 1.0 + 0.35 * (t * 0.23).sin();
            let g2 = (xr * 9.0 * scale - t * 0.5).sin() * (yr * 9.0 * scale + t * 0.45).sin();
            let v = (g1 + g2) * 0.5;
            (clamp01(0.5 + v * 0.7), 0.14 + v * 0.3 + t * 0.05)
        }
        Field::Ripple => {
            let mut v = 0.0f32;
            for index in 0..4 {
                let i = index as f32;
                let px = 1.25 * (t * 0.21 + i * 1.9).sin();
                let py = 0.75 * (t * 0.17 + i * 2.4).cos();
                let d = ((x - px) * (x - px) + (y - py) * (y - py)).sqrt();
                v += (d * 11.0 - t * 3.0 + i).sin() / (1.0 + d * 1.6);
            }
            (clamp01(0.5 + v * 0.42), 0.48 + v * 0.22 + t * 0.03)
        }
    }
}

/// The four phase constants of one metaball, derived from the instance seed.
fn blob_phase(seed: u64, index: usize) -> (f32, f32, f32, f32) {
    let mut rng = SplitMix64::new(seed ^ 0xB10B_0000 ^ index as u64);
    let mut next = || rng.next_range(0.0, f64::from(TAU)) as f32;
    (next(), next(), next(), next())
}

/// Fills the star splat buffer for one frame.
///
/// Hyperspace is the one field that does not read as a function of position: a
/// star is a point that lands on whatever cell it lands on, and several may land
/// on the same one. Accumulating into a buffer is the only honest way to draw
/// that, and it is why [`evaluate_grid`] exists rather than a per-cell closure.
fn splat_stars(
    state: &PhosphorDreamState,
    params: &FieldParams,
    columns: usize,
    rows: usize,
    out: &mut [(f32, f32)],
) {
    for slot in out.iter_mut() {
        *slot = (0.0, 0.0);
    }
    let t = params.time * params.motion;
    let center_x = (columns as f32 - 1.0) * 0.5;
    let center_y = (rows as f32 - 1.0) * 0.5;
    let scale = rows as f32 * 0.5;
    let spin = t * 0.09;
    for index in 0..STAR_COUNT {
        let z = 1.0 - (t * state.star_speed[index] + state.star_depth[index]).rem_euclid(1.0);
        let radius = (1.0 - z).powf(2.3) * 2.6;
        let angle = state.star_angle[index] + spin;
        let sx = radius * angle.cos();
        let sy = radius * angle.sin();
        let column = (center_x + sx * scale / params.cell_aspect).round();
        let row = (center_y + sy * scale).round();
        if !(column >= 0.0 && column < columns as f32 && row >= 0.0 && row < rows as f32) {
            continue;
        }
        let brightness = (1.0 - z).powf(1.4);
        let slot = row as usize * columns + column as usize;
        out[slot].0 += brightness;
        out[slot].1 += state.star_hue[index] * brightness;
    }
}

/// Evaluates the whole glyph field for one frame.
///
/// This is the seam preview and export share. Both call it with the same
/// [`FieldParams`], which is what makes a still frame and the MP4's frame at the
/// same timestamp agree — the invariant PX3 proved at 45 dB for the offline
/// renderer, and the reason field evaluation is not inlined into the draw call.
pub fn evaluate_grid(
    grid: &mut FieldGrid,
    state: &PhosphorDreamState,
    params: &FieldParams,
    columns: usize,
    rows: usize,
) {
    if columns == 0 || rows == 0 {
        grid.resize(0, 0);
        return;
    }
    grid.resize(columns, rows);

    let center_x = (columns as f32 - 1.0) * 0.5;
    let center_y = (rows as f32 - 1.0) * 0.5;
    let scale = rows as f32 * 0.5;

    // Two layers: the incoming field and, during a fade, the outgoing one. Taken
    // as separate passes rather than interleaved so the star splat — which has to
    // see the whole grid — can run for either of them.
    let fill = |field: Field, out: &mut Vec<(f32, f32)>, splat: &mut Vec<(f32, f32)>| {
        if field.is_splatting() {
            splat_stars(state, params, columns, rows, splat);
        }
        for row in 0..rows {
            for column in 0..columns {
                let base_x = (column as f32 - center_x) * params.cell_aspect / scale;
                let base_y = (row as f32 - center_y) / scale;
                let (x, y) = breathe(params, base_x, base_y);
                let slot = row * columns + column;
                out[slot] = if field.is_splatting() {
                    let (accumulated, hue_sum) = splat[slot];
                    // A faint haze so the frame never goes fully dead between
                    // star passes.
                    let haze =
                        0.10 + 0.06 * ((x * x + y * y).sqrt() * 5.0 - params.time * 1.5).sin();
                    let hue = hue_sum / accumulated.max(1e-4) * 0.35 + 0.55 + params.time * 0.02;
                    (clamp01(accumulated) + haze * 0.5, hue)
                } else {
                    evaluate_field(field, state, params, x, y, row, rows, column)
                };
            }
        }
    };

    let mut front = std::mem::take(&mut grid.front);
    let mut back = std::mem::take(&mut grid.back);
    let mut splat = std::mem::take(&mut grid.splat);
    fill(params.field, &mut front, &mut splat);
    if let Some((previous, _)) = params.previous {
        fill(previous, &mut back, &mut splat);
    }

    let ramp = RAMPS[params.ramp.min(RAMP_COUNT - 1)];
    let previous_ramp = params
        .previous
        .map(|(_, index)| RAMPS[index.min(RAMP_COUNT - 1)]);
    let mix = clamp01(params.mix);

    // The brightness shaping, and the audio on top of it. Held to about ±10 % at
    // `reactivity` 1.0 — the source's number and its reasoning, which is worth
    // repeating: a full-frame strobe on every kick at 137 BPM looks cheap and is
    // genuinely unpleasant to sit in front of. The setting goes to 2.0 for anyone
    // who wants it harder; it does not go there by default.
    let pulse = (0.86 + 0.14 * (params.time * 0.29).sin() + 0.06 * (params.time * 0.77).sin())
        * (1.0
            + (0.07 * (params.amplitude - 0.5) + 0.12 * (params.bass - 0.5)) * params.reactivity);

    for row in 0..rows {
        for column in 0..columns {
            let slot = row * columns + column;
            let (mut value, mut hue) = front[slot];
            if let Some(previous) = previous_ramp {
                let (previous_value, previous_hue) = back[slot];
                value = previous_value * (1.0 - mix) + value * mix;
                // A circular hue blend, so a fade sweeps the wheel instead of
                // washing through grey on the way past the seam at 0/1.
                let ca = (previous_hue * TAU).cos() * (1.0 - mix) + (hue * TAU).cos() * mix;
                let sa = (previous_hue * TAU).sin() * (1.0 - mix) + (hue * TAU).sin() * mix;
                hue = sa.atan2(ca) / TAU;
                let _ = previous;
            }

            let vignette = vignette_at(column, row, columns, rows, params.cell_aspect);
            let shaped = clamp01(clamp01(value).powf(1.10) * pulse * vignette);
            let saturation = (0.72 + 0.30 * (hue * 9.0 + params.time * 0.4).sin()).clamp(0.45, 1.0);
            let hue = hue + 0.13 * (params.time * 0.037).sin() + params.hue_shift;

            // The alphabet, dithered rather than blended during a fade: two
            // alphabets cross-dissolved by alpha would just look like both are
            // half-transparent, where a per-cell coin flip weighted by the mix
            // reads as one dissolving into the other. The pattern is a hash of the
            // cell and the seed, so it is stable frame to frame — an animated
            // dither would boil.
            let alphabet = match previous_ramp {
                Some(previous) if cell_dither(params.seed, row, column) >= mix => previous,
                _ => ramp,
            };
            let index = ((shaped * (alphabet.len() - 1) as f32) + 0.5) as usize;
            let ink = alphabet[index.min(alphabet.len() - 1)];

            grid.cells[slot] = GridCell {
                value: shaped,
                hue: hue.rem_euclid(1.0),
                saturation,
                ink,
            };
        }
    }

    grid.front = front;
    grid.back = back;
    grid.splat = splat;
}

/// The per-cell dither threshold, `0..1`.
///
/// Deterministic in the cell and the seed rather than in time. A dither that
/// re-rolled every frame would make the dissolve boil, which reads as noise
/// rather than as a transition.
fn cell_dither(seed: u64, row: usize, column: usize) -> f32 {
    let mut hash = seed
        ^ (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (column as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    hash ^= hash >> 33;
    (hash >> 40) as f32 / (1u32 << 24) as f32
}

/// The corner falloff, so the field reads as a tube rather than a poster.
fn vignette_at(column: usize, row: usize, columns: usize, rows: usize, cell_aspect: f32) -> f32 {
    let center_x = (columns as f32 - 1.0) * 0.5;
    let center_y = (rows as f32 - 1.0) * 0.5;
    let scale = (rows as f32 * 0.5).max(1.0);
    let x = (column as f32 - center_x) * cell_aspect / scale;
    let y = (row as f32 - center_y) / scale;
    (1.15 - 0.42 * (x * x * 0.55 + y * y)).clamp(0.25, 1.0)
}

// -- surfacing words -----------------------------------------------------------

/// Rows of the 5x7 cell used to surface a lyric line into the field.
pub const LETTER_ROWS: usize = 7;
/// Columns of the same.
pub const LETTER_COLUMNS: usize = 5;

/// A compact 5x7 uppercase face, as one `u8` bitmask per row (bit 4 is leftmost).
///
/// Built in rather than rasterized from a TTF for two reasons. The word is drawn
/// *onto the character grid*, so it is already quantized to 5x7-ish at any useful
/// size, and a TTF path would put font rasterization — a raylib call — inside
/// what is otherwise a pure module, which is the boundary that makes this scene
/// testable at all.
///
/// Coverage is A-Z, 0-9 and the punctuation a lyric line actually carries.
/// Anything else falls back to a blank cell rather than a box: an unknown glyph
/// standing out as a rectangle in the middle of a word is worse than a gap.
#[rustfmt::skip]
const LETTERS: [(char, [u8; LETTER_ROWS]); 45] = [
    (' ', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ('A', [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('B', [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
    ('C', [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
    ('D', [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E]),
    ('E', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
    ('F', [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
    ('G', [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F]),
    ('H', [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
    ('I', [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('J', [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
    ('K', [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
    ('L', [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
    ('M', [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11]),
    ('N', [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11]),
    ('O', [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('P', [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
    ('Q', [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
    ('R', [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
    ('S', [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E]),
    ('T', [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('U', [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
    ('V', [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04]),
    ('W', [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11]),
    ('X', [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
    ('Y', [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04]),
    ('Z', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
    ('0', [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
    ('1', [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
    ('2', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F]),
    ('3', [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E]),
    ('4', [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
    ('5', [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
    ('6', [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
    ('7', [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
    ('9', [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C]),
    (',', [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08]),
    ('!', [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04]),
    ('?', [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04]),
    ('\'', [0x04, 0x04, 0x08, 0x00, 0x00, 0x00, 0x00]),
    ('-', [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
    ('~', [0x00, 0x00, 0x08, 0x15, 0x02, 0x00, 0x00]),
    ('*', [0x00, 0x04, 0x15, 0x0E, 0x15, 0x04, 0x00]),
];

/// The bitmap for one character, uppercased. `None` for anything uncovered.
#[must_use]
pub fn letter(character: char) -> Option<[u8; LETTER_ROWS]> {
    let upper = character.to_ascii_uppercase();
    LETTERS
        .iter()
        .find(|(candidate, _)| *candidate == upper)
        .map(|(_, bitmap)| *bitmap)
}

/// How much of `text` fits across `columns` cells, and the scale it fits at.
///
/// Returns the character count that fits and the integer cell scale per letter
/// pixel. A line that cannot fit even at scale 1 is truncated rather than shrunk
/// further, because below one cell per letter pixel the word stops being legible
/// and becomes texture.
#[must_use]
pub fn fit_line(text: &str, columns: usize, rows: usize) -> Option<(usize, usize)> {
    let length = text.chars().filter(|c| letter(*c).is_some()).count();
    if length == 0 || columns == 0 || rows == 0 {
        return None;
    }
    // One blank column between letters.
    let per_letter = LETTER_COLUMNS + 1;
    for scale in (1..=4).rev() {
        let width = length * per_letter * scale;
        let height = LETTER_ROWS * scale;
        if width <= columns && height <= rows {
            return Some((length, scale));
        }
    }
    let fits = columns / per_letter;
    (fits > 0 && LETTER_ROWS <= rows).then_some((fits, 1))
}

/// Whether a grid cell falls on the ink of a surfaced line.
///
/// Centred both ways. Used by the drawing half to lift the letters out of the
/// field: the source dims everything else, brightens the letters and drops their
/// saturation to white, which is what makes a word *surface* rather than sit on
/// top.
#[must_use]
pub fn line_ink(text: &str, columns: usize, rows: usize, row: usize, column: usize) -> bool {
    let Some((count, scale)) = fit_line(text, columns, rows) else {
        return false;
    };
    let per_letter = (LETTER_COLUMNS + 1) * scale;
    let width = count * per_letter;
    let height = LETTER_ROWS * scale;
    let left = (columns.saturating_sub(width)) / 2;
    let top = (rows.saturating_sub(height)) / 2;
    if column < left || row < top || column >= left + width || row >= top + height {
        return false;
    }
    let local_x = column - left;
    let local_y = row - top;
    let letter_index = local_x / per_letter;
    let inside_x = (local_x % per_letter) / scale;
    if inside_x >= LETTER_COLUMNS {
        return false;
    }
    let inside_y = local_y / scale;
    let Some(character) = text
        .chars()
        .filter(|c| letter(*c).is_some())
        .nth(letter_index)
    else {
        return false;
    };
    let Some(bitmap) = letter(character) else {
        return false;
    };
    bitmap[inside_y.min(LETTER_ROWS - 1)] & (1 << (LETTER_COLUMNS - 1 - inside_x)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneSettings, SCENE_COUNT};

    fn settings() -> SceneSettings {
        SceneSettings::default()
    }

    #[test]
    fn the_bank_and_the_descriptor_bounds_agree() {
        // The `field` setting says "0 = cycle, 1..=maximum pins one". If the bank
        // grew without the descriptor growing, the last field would be
        // unreachable from the inspector and nothing else would complain.
        let descriptors = crate::scene::settings::descriptors(SceneId::PhosphorDream);
        assert_eq!(
            descriptors[setting::FIELD].maximum as usize,
            FIELD_COUNT,
            "the field selector must reach every field in the bank"
        );
        assert_eq!(
            descriptors[setting::RAMP].maximum as usize,
            RAMP_COUNT,
            "the alphabet selector must reach every ramp"
        );
        assert_eq!(Field::ALL.len(), FIELD_COUNT);
        assert_eq!(RAMPS.len(), RAMP_COUNT);
        assert_eq!(RAMP_NAMES.len(), RAMP_COUNT);
        // Twelve controls is exactly MAX_CONTROLS; a thirteenth would silently be
        // unreachable through the dense value array rather than failing to build.
        assert!(descriptors.len() <= crate::scene::settings::MAX_CONTROLS);
        assert_eq!(SCENE_COUNT, 11);
    }

    /// Every `Ink::Char` must be inside raylib's built-in face, which is Latin-1.
    ///
    /// This is the test that keeps the alphabets honest. A `Char('█')` would
    /// compile, draw as an empty box, and look like a field with holes in it —
    /// a plausible picture, which is the failure mode this repository keeps
    /// paying for. Anything outside the range has to be an `Ink::Shape`.
    #[test]
    fn every_lettered_ramp_entry_is_inside_the_built_in_face() {
        for (index, ramp) in RAMPS.iter().enumerate() {
            assert!(!ramp.is_empty(), "{} is empty", RAMP_NAMES[index]);
            for ink in ramp.iter() {
                if let Some(codepoint) = ink.codepoint() {
                    assert!(
                        (0x20..=0xFF).contains(&codepoint),
                        "{}: U+{codepoint:04X} is outside the built-in face; \
                         it has to be an Ink::Shape",
                        RAMP_NAMES[index],
                    );
                }
            }
        }
    }

    #[test]
    fn every_ramp_starts_blank_so_a_dark_field_is_actually_dark() {
        for (index, ramp) in RAMPS.iter().enumerate() {
            assert!(
                ramp[0].is_blank(),
                "{} does not start blank, so value 0 would still put ink down",
                RAMP_NAMES[index]
            );
            assert!(
                !ramp[ramp.len() - 1].is_blank(),
                "{} ends blank, so its brightest cell would draw nothing",
                RAMP_NAMES[index]
            );
        }
    }

    #[test]
    fn the_follower_attacks_instantly_and_releases_slowly() {
        // Attack: straight to the target, whatever the delta.
        assert_eq!(PhosphorDreamState::follow(0.0, 1.0, 0.016), 1.0);
        // Release: the source's per-frame 0.86 at 30 fps, reproduced at 1/30 s.
        let after = PhosphorDreamState::follow(1.0, 0.0, 1.0 / 30.0);
        assert!(
            (after - 0.86).abs() < 1.0e-3,
            "one 30 fps frame of release should retain ~0.86, got {after}"
        );
        // And the same decay over the same wall time at 60 fps — two frames.
        let a = PhosphorDreamState::follow(1.0, 0.0, 1.0 / 60.0);
        let b = PhosphorDreamState::follow(a, 0.0, 1.0 / 60.0);
        assert!(
            (b - after).abs() < 1.0e-3,
            "60 fps must decay at the same rate per second as 30 fps: {b} vs {after}"
        );
        // A zero or negative delta cannot move it, so a paused frame does not
        // decay the envelope to nothing.
        assert_eq!(PhosphorDreamState::follow(0.5, 0.0, 0.0), 0.5);
        assert_eq!(PhosphorDreamState::follow(0.5, 0.0, -1.0), 0.5);
        assert_eq!(PhosphorDreamState::follow(0.5, f32::NAN, 0.1), 0.5);
    }

    #[test]
    fn the_cycle_advances_on_the_dwell_and_pass_zero_is_the_default_order() {
        let state = PhosphorDreamState::new(7);
        for slot in 0..FIELD_COUNT {
            let (field, ramp) = state.slot_program(slot);
            assert_eq!(field, Field::ALL[slot]);
            assert_eq!(ramp, field.default_ramp());
        }
        // Later passes are a permutation, not the same order again.
        let first: Vec<Field> = (0..FIELD_COUNT).map(|s| state.slot_program(s).0).collect();
        let second: Vec<Field> = (FIELD_COUNT..2 * FIELD_COUNT)
            .map(|s| state.slot_program(s).0)
            .collect();
        assert_ne!(first, second, "pass 1 should be reshuffled");
        let mut sorted_first = first.clone();
        let mut sorted_second = second.clone();
        sorted_first.sort_by_key(|f| f.name());
        sorted_second.sort_by_key(|f| f.name());
        assert_eq!(
            sorted_first, sorted_second,
            "a pass must be a permutation — every field exactly once"
        );
    }

    #[test]
    fn a_pinned_field_stops_the_clock_rather_than_letting_it_run_invisibly() {
        let mut values = settings();
        assert!(values.set(SceneId::PhosphorDream, setting::FIELD, 3.0));
        let mut state = PhosphorDreamState::new(1);
        let mut frame = SceneFrame::idle(&values);
        frame.delta_seconds = 1.0;
        for _ in 0..60 {
            state.update(&frame);
        }
        assert_eq!(
            state.slot(),
            0,
            "a pinned field must not advance the cycle underneath the user"
        );
        let params = FieldParams::resolve(&state, &frame, 0.6);
        assert_eq!(params.field, Field::ALL[2], "field 3 is the third entry");
        assert!(params.previous.is_none(), "a pinned field never crossfades");
    }

    #[test]
    fn the_dwell_governs_the_cycle_and_the_fade_is_a_fraction_of_it() {
        let mut values = settings();
        assert!(values.set(SceneId::PhosphorDream, setting::DWELL, 10.0));
        let mut state = PhosphorDreamState::new(1);
        let mut frame = SceneFrame::idle(&values);
        frame.delta_seconds = 0.25;
        // 10 s of frames is exactly one dwell.
        for _ in 0..40 {
            state.update(&frame);
        }
        assert_eq!(state.slot(), 1);
        // The fade is capped by the dwell, so a 4 s dwell does not spend 2.6 s
        // dissolving.
        assert_eq!(PhosphorDreamState::fade_seconds(10.0), FADE_SECONDS);
        assert!((PhosphorDreamState::fade_seconds(4.0) - 1.04).abs() < 1.0e-5);
    }

    #[test]
    fn a_long_stall_cannot_skip_a_whole_field() {
        let mut state = PhosphorDreamState::new(1);
        let values = settings();
        let mut frame = SceneFrame::idle(&values);
        // A ten-second hitch: a seek, a debugger, a slow export step.
        frame.delta_seconds = 10.0;
        state.update(&frame);
        assert_eq!(
            state.slot(),
            0,
            "a stall is capped at 0.25 s so the cycle does not jump"
        );
        assert!(state.slot_seconds() <= 0.25);
    }

    #[test]
    fn evaluation_is_deterministic_and_bounded() {
        let values = settings();
        let mut state = PhosphorDreamState::new(99);
        let mut frame = SceneFrame::idle(&values);
        frame.delta_seconds = 1.0 / 60.0;
        frame.time_seconds = 12.5;
        state.update(&frame);
        let params = FieldParams::resolve(&state, &frame, 0.6);

        let mut a = FieldGrid::new();
        let mut b = FieldGrid::new();
        evaluate_grid(&mut a, &state, &params, 48, 20);
        evaluate_grid(&mut b, &state, &params, 48, 20);
        assert_eq!(a.columns(), 48);
        assert_eq!(a.rows(), 20);
        for row in 0..20 {
            for column in 0..48 {
                let left = a.cell(row, column).unwrap();
                let right = b.cell(row, column).unwrap();
                assert_eq!(left, right, "evaluation must be deterministic");
                assert!(
                    (0.0..=1.0).contains(&left.value),
                    "value out of range at {row},{column}: {}",
                    left.value
                );
                assert!((0.0..=1.0).contains(&left.hue));
                assert!((0.0..=1.0).contains(&left.saturation));
            }
        }
    }

    /// Every field has to produce a picture, and the cheap way to be wrong here
    /// is to produce a *uniform* one — a field that evaluates to the same value
    /// everywhere still draws a plausible frame of flat texture.
    #[test]
    fn every_field_draws_something_with_structure_in_it() {
        let mut values = settings();
        let mut state = PhosphorDreamState::new(4);
        for index in 0..FIELD_COUNT {
            assert!(values.set(SceneId::PhosphorDream, setting::FIELD, index as f32 + 1.0));
            let mut frame = SceneFrame::idle(&values);
            frame.time_seconds = 3.0;
            frame.delta_seconds = 1.0 / 60.0;
            state.update(&frame);
            let params = FieldParams::resolve(&state, &frame, 0.6);
            assert_eq!(params.field, Field::ALL[index]);

            let mut grid = FieldGrid::new();
            evaluate_grid(&mut grid, &state, &params, 64, 28);
            let mut minimum = f32::MAX;
            let mut maximum = f32::MIN;
            let mut inked = 0usize;
            for row in 0..28 {
                for column in 0..64 {
                    let cell = grid.cell(row, column).unwrap();
                    minimum = minimum.min(cell.value);
                    maximum = maximum.max(cell.value);
                    if !cell.ink.is_blank() {
                        inked += 1;
                    }
                }
            }
            let field = Field::ALL[index];
            assert!(
                maximum - minimum > 0.05,
                "{}: flat field, range {minimum}..{maximum}",
                field.name()
            );
            assert!(
                inked > 32,
                "{}: only {inked} inked cells — the frame is nearly empty",
                field.name()
            );
        }
    }

    #[test]
    fn a_crossfade_blends_both_fields_and_dithers_both_alphabets() {
        let mut values = settings();
        assert!(values.set(SceneId::PhosphorDream, setting::DWELL, 10.0));
        let mut state = PhosphorDreamState::new(3);
        let mut frame = SceneFrame::idle(&values);
        frame.delta_seconds = 0.25;
        // Land one second into slot 1, inside the 2.6 s fade.
        for _ in 0..44 {
            state.update(&frame);
        }
        assert_eq!(state.slot(), 1);
        let params = FieldParams::resolve(&state, &frame, 0.6);
        let (previous_field, previous_ramp) = params.previous.expect("a fade should be running");
        assert_eq!(previous_field, Field::ALL[0]);
        assert!(params.mix > 0.0 && params.mix < 1.0, "mix {}", params.mix);

        // Both alphabets must appear on the grid during the dissolve, otherwise
        // the dither is not doing anything and the fade is a plain blend.
        let mut grid = FieldGrid::new();
        evaluate_grid(&mut grid, &state, &params, 64, 28);
        let current = RAMPS[params.ramp];
        let previous = RAMPS[previous_ramp];
        assert_ne!(
            params.ramp, previous_ramp,
            "the two slots share an alphabet"
        );
        let mut from_current = 0usize;
        let mut from_previous = 0usize;
        for row in 0..28 {
            for column in 0..64 {
                let ink = grid.cell(row, column).unwrap().ink;
                if ink.is_blank() {
                    continue;
                }
                if current.contains(&ink) && !previous.contains(&ink) {
                    from_current += 1;
                }
                if previous.contains(&ink) && !current.contains(&ink) {
                    from_previous += 1;
                }
            }
        }
        assert!(
            from_current > 0 && from_previous > 0,
            "the dissolve should show both alphabets: {from_current} / {from_previous}"
        );
    }

    #[test]
    fn the_dither_is_stable_in_time_so_a_dissolve_does_not_boil() {
        // Same cell, same seed, twice: the threshold is a function of position
        // only. An animated one would make the crossfade look like static.
        for row in 0..8 {
            for column in 0..8 {
                let a = cell_dither(11, row, column);
                let b = cell_dither(11, row, column);
                assert_eq!(a, b);
                assert!((0.0..1.0).contains(&a), "{a} out of range");
            }
        }
        // And it is not constant across the grid, which would make the dissolve
        // a hard cut.
        let values: Vec<f32> = (0..64).map(|i| cell_dither(11, i / 8, i % 8)).collect();
        let spread = values.iter().cloned().fold(f32::MIN, f32::max)
            - values.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.5, "dither spread too narrow: {spread}");
    }

    #[test]
    fn a_surfaced_line_is_centred_and_only_covers_its_own_letters() {
        let columns = 80;
        let rows = 30;
        let (count, scale) = fit_line("HI", columns, rows).unwrap();
        assert_eq!(count, 2);
        assert!(scale >= 1);
        let mut inked = 0;
        let mut min_column = usize::MAX;
        let mut max_column = 0usize;
        for row in 0..rows {
            for column in 0..columns {
                if line_ink("HI", columns, rows, row, column) {
                    inked += 1;
                    min_column = min_column.min(column);
                    max_column = max_column.max(column);
                }
            }
        }
        assert!(inked > 0, "nothing was drawn");
        // Centred: the gaps either side should be within a cell of each other.
        let left_gap = min_column;
        let right_gap = columns - 1 - max_column;
        assert!(
            left_gap.abs_diff(right_gap) <= scale * (LETTER_COLUMNS + 1),
            "not centred: {left_gap} vs {right_gap}"
        );
        // An empty or uncovered line puts nothing down at all.
        assert!(fit_line("", columns, rows).is_none());
        assert!(!line_ink("", columns, rows, rows / 2, columns / 2));
        assert!(fit_line("日本語", columns, rows).is_none());
    }

    #[test]
    fn a_zero_sized_grid_is_refused_rather_than_panicking() {
        let values = settings();
        let state = PhosphorDreamState::new(1);
        let frame = SceneFrame::idle(&values);
        let params = FieldParams::resolve(&state, &frame, 0.6);
        let mut grid = FieldGrid::new();
        evaluate_grid(&mut grid, &state, &params, 0, 20);
        assert_eq!(grid.columns(), 0);
        assert!(grid.cell(0, 0).is_none());
    }

    /// The bass reading must be the *same* number the caption glow is driven by.
    ///
    /// This is the regression test for two definitions of one word. It asserts
    /// equality with `EffectInputs` rather than re-deriving the quarter here,
    /// because a copy of the formula in the assertion would drift with the copy
    /// in the code and agree with it while both were wrong.
    #[test]
    fn the_bass_reading_is_the_applications_one_definition_of_bass() {
        use crate::project::caption_effects::EffectInputs;

        let empty = SceneAudioFrame::default();
        assert_eq!(PhosphorDreamState::raw_bass(&empty), 0.0);

        // Sixteen bands whose *trails* carry the low energy. The instantaneous
        // bands are deliberately the other way round: a reader of `bands` would
        // report 0.0 here, which is exactly the bug this replaced.
        let bands = [0.0f32; 16];
        let mut trails = [0.0f32; 16];
        for slot in trails.iter_mut().take(4) {
            *slot = 0.8;
        }
        let audio = SceneAudioFrame::from_spectrum(&bands, &trails);
        let expected = EffectInputs::from_audio(0.0, audio.rms, audio.trails, 0.0, 0.0).bass;
        assert!(
            (expected - 0.8).abs() < 1.0e-6,
            "fixture is wrong: {expected}"
        );
        assert_eq!(
            PhosphorDreamState::raw_bass(&audio),
            expected,
            "the scene and the caption glow must read the same bass"
        );
    }

    #[test]
    fn the_state_downcasts_from_the_registry_and_reports_its_own_id() {
        use crate::scene::SceneInstance;
        let mut instance = SceneInstance::new(descriptor(), 5);
        assert_eq!(instance.id(), SceneId::PhosphorDream);
        let values = settings();
        let mut frame = SceneFrame::idle(&values);
        frame.delta_seconds = 1.0 / 60.0;
        instance.update(&frame);
        let state = instance
            .state()
            .as_any()
            .downcast_ref::<PhosphorDreamState>()
            .expect("the descriptor and the state type must agree");
        assert_eq!(state.seed(), 5);
        instance.rebind();
        assert_eq!(instance.seed(), 5);
    }
}
