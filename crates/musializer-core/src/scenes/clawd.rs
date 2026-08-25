//! Clawd: deterministic state for the flower who listens.
//!
//! **No oracle.** Like Phosphor Dream, this is a post-legacy addition
//! (2026-08-24) under the rule in `AGENTS.md` that features past the frozen C's
//! ceiling need no parity justification. There is nothing to cite and nothing
//! to diff against, so the evidence for this file is its own tests.
//!
//! ## What it is
//!
//! The twelve-petalled flower character drawn by **thebes**
//! (<https://x.com/voooooogel>), reimagined as a music visualizer: each petal is
//! one of twelve spectrum band groups and plumps with its band, the head does
//! squash-and-stretch on the beat, and the face is an expression state machine
//! driven by the analysis — a resting smile, a scrunch on hard onsets, spirals
//! under sustained flux, pleading eyes in long quiet, and a rare head-explode on
//! a genuine peak. Kaomoji cats bounce along the floor, a smoke ribbon rises
//! when the energy sustains, and an optional terminal line types the track's
//! own lyric cue. The scene is an homage, built procedurally rather than from
//! copied artwork; the drawing half owns every stroke.
//!
//! ## Design decisions a later reader will otherwise re-litigate
//!
//! - **The face never spins.** The petal ring rotates; the face stays upright.
//!   A face is the one element with an orientation a viewer reads pre-attentively,
//!   and a rotating smile reads as a rendering fault, not a dance.
//! - **Expressions are a state machine with dwell times, not a per-frame map.**
//!   Mapping flux to a face directly flickers at frame rate — the machine gives
//!   each expression a minimum residency so the character appears to *react*,
//!   not strobe. `mood` scales the entry thresholds; zero pins the resting
//!   smile, which keeps "the machine is dead" and "the operator asked for calm"
//!   distinguishable states (the report line names the current face either way).
//! - **The boom has a cooldown.** A head-explode that fires on every drop is
//!   wallpaper by the second chorus. One per ~22 s keeps it an event.
//! - **The terminal types the track's own cue** and defaults off, exactly like
//!   phosphor's `titles`: this application has a caption layer with authored
//!   timing, and two word layers fighting for one frame is the wrong default.
//! - **Typing advances even while the toggle is off.** The counter is a few
//!   additions per frame, and advancing it unconditionally means flipping the
//!   toggle mid-cue shows a line typed to where the cue *is*, not a line that
//!   starts typing from nothing — the toggle gates drawing, not time.
//! - **The semantic lane warms the character rather than steering it.**
//!   `warmth` is a slow follower over the Assist lane's valence; the drawing
//!   half reads it to soften the resting face and petal saturation, so two
//!   tracks at the same RMS read differently when the lane knows their mood.
//!   Tension biases the expression thresholds instead of adding a sixth face:
//!   a tense track should scrunch easier at the same slider setting, and a new
//!   expression would need strokes, a dwell and a report token for what is
//!   really just sensitivity.
//! - **Blinks exist because nothing sells "alive" cheaper.** A face that never
//!   blinks reads as a sticker within seconds; a 0.24 s eyelid every few
//!   seconds fixes that for the cost of one countdown. The schedule is seeded,
//!   not wall-clock jittered, because export determinism is a hard contract —
//!   the same seed and frames must render the same eyelids in every export.
//! - **The mouth sings the captions.** While a cue is live the resting `w`
//!   interpolates toward an `o` on a flux-driven, syllable-rate oscillator, so
//!   the character visibly sings its own lyric line rather than mouthing a
//!   static smile under moving text.
//!
//! Everything here is pure: no clock, no I/O, and the only randomness is derived
//! from the instance seed.

use std::any::Any;

use crate::project::caption_effects::bass_from_trails;
use crate::scene::settings::index::clawd as setting;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use crate::ui::tune_explore::{RandomSource, SplitMix64};

/// Bumped when the state layout changes so a rebind discards stale state.
/// 2 (2026-08-24): the senses wave — semantic warmth, the sing-along mouth,
/// and the seeded blink schedule.
pub const STATE_VERSION: u32 = 3;

/// The petal count is the character's anatomy, not a tunable: thebes draws the
/// flower with twelve petals, and twelve conveniently divides any analyzer band
/// count into contiguous groups. Changing it changes who this is.
pub const PETAL_COUNT: usize = 12;

/// Upper bound of the `cats` setting, and the size of the per-cat seed tables.
/// Asserted equal to the descriptor maximum in the tests.
pub const MAX_CATS: usize = 6;

/// How long a head-explode plays out. Long enough to read the mushroom cloud,
/// short enough that the petals are back before the next bar.
pub const BOOM_SECONDS: f32 = 1.6;

/// Minimum seconds between booms. Below this an energetic track keeps the head
/// permanently exploding and the event stops being one.
pub const BOOM_COOLDOWN_SECONDS: f32 = 22.0;

/// How long a scrunch (`>` `<`) holds after the onset that triggered it.
/// Roughly a hard blink; shorter reads as a glitch frame.
pub const SCRUNCH_SECONDS: f32 = 0.26;

/// Characters per second the lyric terminal types. Brisk enough to finish a
/// normal line well inside its cue, slow enough to read as typing.
pub const TYPE_CHARS_PER_SECOND: f32 = 28.0;

/// Per-second retention of the amplitude/bass envelope release limb — the same
/// value Phosphor Dream uses, for the same reason: expressed per second so
/// preview and export agree at any frame rate.
const ENVELOPE_RELEASE_PER_SECOND: f32 = 0.010_69;

/// Petals release faster than the aggregate envelopes: a petal is a bar, and a
/// bar that lingers a second behind its band reads as decoration rather than
/// measurement.
const PETAL_RELEASE_PER_SECOND: f32 = 0.000_8;

/// Retention of the beat-bounce impulse. `0.002^0.3 ≈ 0.15`, so a bounce is
/// visually over ~0.3 s after the beat — a bob, not a wobble.
const BOUNCE_RELEASE_PER_SECOND: f32 = 0.002;

/// The bass-transient ("kick") gate, on the mean positive excursion of the
/// lowest quarter of the bands over their own trails — the same shape as
/// spectral flux, scoped to the region `bass_from_trails` calls bass.
///
/// **Measured, not guessed** (2026-08-25, `examples/cat_probe.rs` against the
/// operator's *Parameter People*): at 0.05 the detector finds the track's real
/// bass groove (~0.33 s spacing where one exists) and stays honestly quiet
/// through its pad sections, where 0.03 chatters and 0.08 misses all but the
/// section hits. The first version hopped the cats on beat-tracker phase wraps
/// instead, and on that track the tracker anchors only at four section
/// transitions in ten seconds and freewheels between them — a metronome nobody
/// is conducting, which is exactly the "jumps that never land" the operator
/// reported.
pub const KICK_THRESHOLD: f32 = 0.05;

/// Minimum seconds between kicks. A sustained excursion is one push, not a
/// drumroll; 0.25 s admits a 16th-note groove at 60 BPM and nothing sillier.
const KICK_REFRACTORY_SECONDS: f32 = 0.25;

/// A hop is a fixed ballistic arc — `sin(pi * age / HOP_SECONDS)` — not a
/// decaying envelope. The first version teleported to apex and oozed down
/// asymptotically, which reads as floating; an arc has a takeoff and, more to
/// the point, a landing.
pub const HOP_SECONDS: f32 = 0.34;

/// Widest per-cat seeded takeoff stagger. Even a party hop is not robotic
/// unison; 60 ms is audible tightness, visible individuality.
const HOP_STAGGER_MAX_SECONDS: f32 = 0.06;

/// A kick at least this strong, with [`PARTY_AMPLITUDE`], sends every cat up.
const PARTY_KICK: f32 = 0.10;

/// The loudness gate on the party rule. The first version partied on
/// `amplitude > 0.75` at every beat wrap, which on any loud track meant the
/// take-turns choreography never appeared at all — three cats pogoing in
/// lockstep to a freewheeling clock.
const PARTY_AMPLITUDE: f32 = 0.85;

/// What a tracker phase wrap is still worth: a soft head bob, never a cat.
/// The tracker freewheels between anchors (see [`KICK_THRESHOLD`]), and a
/// character slamming to an extrapolated grid is the defect this replaced —
/// but a gentle nod keeps the flower breathing on tracks where the grid is
/// real, and on those the kicks land on top of it anyway.
const WRAP_BOUNCE: f32 = 0.35;

/// Amplitude below which the frame counts toward "quiet" for the pleading face.
const QUIET_AMPLITUDE: f32 = 0.06;

/// Amplitude that ends a pleading spell.
const RECOVER_AMPLITUDE: f32 = 0.12;

/// Flux envelope above which the frame counts toward "busy" for the dizzy face.
const BUSY_FLUX: f32 = 0.5;

/// Flux envelope below which a dizzy spell ends.
const CALM_FLUX: f32 = 0.3;

/// A stalled frame must not advance the character by a whole phrase — the same
/// cap and the same reasoning as Phosphor Dream's cycle clock.
const MAX_STEP_SECONDS: f32 = 0.25;

/// Confidence below which the semantic lane is treated as absent. The lane's
/// own trust policy (see `docs/ASSIST_PIPELINE.md`) hands low-confidence
/// interpretations to the UI for review, not to a renderer to act on.
pub const SEMANTIC_CONFIDENCE_FLOOR: f32 = 0.35;

/// Where warmth rests with no usable semantic lane. Slightly warm rather than
/// a dead-centre 0.5 because the character's *resting design* is friendly —
/// neutral here means "the face thebes drew", not "no opinion".
pub const NEUTRAL_WARMTH: f32 = 0.6;

/// Warmth's per-second retention: `e^-0.5`, a ~2 s time constant. Mood is a
/// track-scale quantity; a warmth that chased valence at envelope speed would
/// flicker the palette, which reads as a rendering fault rather than feeling.
const WARMTH_RETENTION_PER_SECOND: f32 = 0.606_5;

/// How hard tension leans on the expression thresholds at full confidence.
/// 0.6 lowers the scrunch bar by ~38 % — enough that a tense track visibly
/// reacts more, not so much that every frame becomes an event.
const TENSION_MOOD_GAIN: f32 = 0.6;

/// The `mood` descriptor's maximum, which the tension bias may not exceed:
/// the threshold clamps in [`ClawdState::step_expression`] were written
/// against what the slider itself can ask for. Asserted against the
/// descriptor in the tests so the two cannot drift apart.
const MOOD_CEILING: f32 = 2.0;

/// A blink's full playout — a triangular envelope, closed at the midpoint.
/// Human blinks run 0.1–0.4 s; 0.24 s reads as a blink at 60 fps and still
/// spans several frames at export rates.
pub const BLINK_SECONDS: f32 = 0.24;

/// The blink interval is drawn uniformly from this range and re-drawn after
/// each blink. 2.5..6.5 s straddles the human resting rate; a fixed interval
/// reads as a metronome, which is the opposite of alive.
const BLINK_INTERVAL_MIN_SECONDS: f32 = 2.5;
const BLINK_INTERVAL_MAX_SECONDS: f32 = 6.5;

/// The sing-along carrier's base rate. ~5.5 Hz sits in the syllable band of
/// sung speech, so the mouth flaps at lyric speed rather than at frame rate.
const SING_RATE_HZ: f32 = 5.5;

/// Flux-to-mouth gain: a clear onset (flux env ~0.5) opens the mouth most of
/// the way before the carrier modulates it.
const MOUTH_FLUX_GAIN: f32 = 1.6;

/// Mouth attack retention: `e^-12.5` per second, a ~80 ms time constant —
/// fast enough to track the syllable carrier, slow enough not to alias it.
const MOUTH_ATTACK_RETENTION_PER_SECOND: f32 = 0.000_003_7;

/// Mouth release retention with no cue: `e^-6.67` per second, ~150 ms. The
/// mouth closes over a couple of frames rather than snapping shut when a cue
/// ends mid-word.
const MOUTH_RELEASE_RETENTION_PER_SECOND: f32 = 0.001_27;

// -- expressions ---------------------------------------------------------------

/// The face. Five states, drawn from thebes' expression sheet; the drawing half
/// owns the strokes, this enum owns *when*.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Expression {
    /// The resting `^ᴗ^`-and-`w` smile.
    #[default]
    Happy,
    /// `>` `<` — a hard blink on a hard onset.
    Scrunch,
    /// Spiral eyes under sustained flux.
    Dizzy,
    /// Big wet eyes after a long quiet stretch.
    Pleading,
    /// The mushroom cloud. Rare on purpose; see [`BOOM_COOLDOWN_SECONDS`].
    Boom,
}

impl Expression {
    /// The token the report line prints. Lowercase and stable — the headless
    /// gate greps for these.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Expression::Happy => "happy",
            Expression::Scrunch => "scrunch",
            Expression::Dizzy => "dizzy",
            Expression::Pleading => "pleading",
            Expression::Boom => "boom",
        }
    }
}

// -- cats ----------------------------------------------------------------------

/// What a cat's face is doing. Derived, not stored: a cat has no memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatFace {
    /// `( ^w^ )`
    Content,
    /// `( >w< )` — the track is loud and the cat approves.
    Excited,
    /// `( -w- )` — nothing has happened for a while.
    Sleepy,
    /// `( o.o )` — one seeded cat is always this cat.
    Curious,
}

/// One cat, resolved for the current frame. Everything is in scene-relative
/// units so the drawing half owns pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CatPose {
    /// Horizontal position across the floor, `0..1`.
    pub lane: f32,
    /// Hop lift, `0..1` of a cat height.
    pub hop: f32,
    /// Size jitter around `1.0` so the row reads as individuals, not tiling.
    pub scale: f32,
    /// Whether the cat faces left.
    pub flip: bool,
    pub face: CatFace,
}

// -- state ---------------------------------------------------------------------

/// The whole deterministic state. Everything the drawing half reads comes from
/// here or from the frame; nothing is derived twice.
pub struct ClawdState {
    seed: u64,

    // Envelopes.
    amplitude: f32,
    bass: f32,
    flux: f32,
    petals: [f32; PETAL_COUNT],

    // Motion.
    petal_angle: f32,
    sway_phase: f32,
    bounce: f32,
    beat_count: u64,
    previous_beat_phase: f32,

    // The face.
    expression: Expression,
    expression_age: f32,
    quiet_seconds: f32,
    busy_seconds: f32,
    boom_cooldown: f32,

    // The senses wave (STATE_VERSION 2).
    warmth: f32,
    semantic_active: bool,
    mouth_open: f32,
    sing_phase: f32,
    blink_rng: SplitMix64,
    blink_countdown: f32,
    blink_age: f32,

    // Atmosphere.
    smoke: f32,

    // The bass-transient detector (STATE_VERSION 3).
    kick_refractory: f32,
    kick_count: u64,
    last_kick: f32,

    // Cats. The jitter tables are seed-derived at construction so a cat keeps
    // its personality across the whole track. A hop is a clock, not an
    // envelope: `hop_age` runs from `-stagger` through [`HOP_SECONDS`] and the
    // pose derives the arc from it.
    cat_count: usize,
    hop_age: [f32; MAX_CATS],
    hop_power: [f32; MAX_CATS],
    hop_stagger: [f32; MAX_CATS],
    cat_lane_jitter: [f32; MAX_CATS],
    cat_scale_jitter: [f32; MAX_CATS],
    cat_flip: [bool; MAX_CATS],
    curious_cat: usize,

    // The lyric terminal.
    typed: f32,
    typed_cue: Option<u64>,
    typed_limit: usize,
}

impl ClawdState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut random = SplitMix64::new(seed ^ 0xC1AD_0000_F10E_0011);
        let mut cat_lane_jitter = [0.0f32; MAX_CATS];
        let mut cat_scale_jitter = [0.0f32; MAX_CATS];
        let mut cat_flip = [false; MAX_CATS];
        for i in 0..MAX_CATS {
            cat_lane_jitter[i] = random.next_range(-1.0, 1.0) as f32;
            cat_scale_jitter[i] = random.next_range(0.85, 1.15) as f32;
            cat_flip[i] = random.chance(0.5);
        }
        let curious_cat = (random.next_u64() % MAX_CATS as u64) as usize;
        // Drawn after the tables above so their values are unchanged from
        // STATE_VERSION 2 — the stagger is a new draw appended to the stream.
        let mut hop_stagger = [0.0f32; MAX_CATS];
        for stagger in &mut hop_stagger {
            *stagger = random.next_range(0.0, f64::from(HOP_STAGGER_MAX_SECONDS)) as f32;
        }
        // Blinks get their own generator and domain constant rather than
        // sharing `random`: a shared stream would make the blink schedule move
        // whenever the cat tables gain or lose a draw, which is exactly the
        // cross-coupling a rebind's determinism contract exists to prevent.
        let mut blink_rng = SplitMix64::new(seed ^ 0xC1AD_0000_B11E_0022);
        let blink_countdown = blink_rng.next_range(
            f64::from(BLINK_INTERVAL_MIN_SECONDS),
            f64::from(BLINK_INTERVAL_MAX_SECONDS),
        ) as f32;
        Self {
            seed,
            amplitude: 0.0,
            bass: 0.0,
            flux: 0.0,
            petals: [0.0; PETAL_COUNT],
            petal_angle: 0.0,
            sway_phase: 0.0,
            bounce: 0.0,
            beat_count: 0,
            previous_beat_phase: 0.0,
            expression: Expression::Happy,
            expression_age: 0.0,
            quiet_seconds: 0.0,
            busy_seconds: 0.0,
            boom_cooldown: 0.0,
            warmth: NEUTRAL_WARMTH,
            semantic_active: false,
            mouth_open: 0.0,
            sing_phase: 0.0,
            blink_rng,
            blink_countdown,
            // No blink in flight: the age starts past the envelope's end.
            blink_age: BLINK_SECONDS,
            smoke: 0.0,
            kick_refractory: 0.0,
            kick_count: 0,
            last_kick: 0.0,
            cat_count: 0,
            // Landed: every age starts past the arc's end.
            hop_age: [HOP_SECONDS; MAX_CATS],
            hop_power: [0.0; MAX_CATS],
            hop_stagger,
            cat_lane_jitter,
            cat_scale_jitter,
            cat_flip,
            curious_cat,
            typed: 0.0,
            typed_cue: None,
            typed_limit: 0,
        }
    }

    // -- getters the drawing half reads ---------------------------------------

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Fast-attack, slow-release amplitude envelope, `0..1`.
    #[must_use]
    pub fn amplitude(&self) -> f32 {
        self.amplitude
    }

    /// Same envelope over the application's one definition of bass.
    #[must_use]
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// Same envelope over spectral flux, scaled into `0..1`.
    #[must_use]
    pub fn flux(&self) -> f32 {
        self.flux
    }

    /// Per-petal band-group energy, `0..1` each, index `0..PETAL_COUNT`.
    #[must_use]
    pub fn petals(&self) -> &[f32; PETAL_COUNT] {
        &self.petals
    }

    /// Petal ring rotation in radians. The face does not read this.
    #[must_use]
    pub fn petal_angle(&self) -> f32 {
        self.petal_angle
    }

    /// The idle-sway clock. Monotonic; drawn through sines by the app half.
    /// Never wrapped — f32 keeps sub-frame precision far past any track length,
    /// and wrapping would kink every non-unit harmonic read from it.
    #[must_use]
    pub fn sway_phase(&self) -> f32 {
        self.sway_phase
    }

    /// Beat squash-and-stretch impulse, `1.0` at the beat decaying to `0`.
    #[must_use]
    pub fn bounce(&self) -> f32 {
        self.bounce
    }

    /// Beats seen so far. The report line prints it: a track with music and a
    /// count of zero means the beat tracker never coupled.
    #[must_use]
    pub fn beat_count(&self) -> u64 {
        self.beat_count
    }

    #[must_use]
    pub fn expression(&self) -> Expression {
        self.expression
    }

    /// Seconds in the current expression.
    #[must_use]
    pub fn expression_age(&self) -> f32 {
        self.expression_age
    }

    /// `0..1` through a boom's playout, `0` for every other face. The drawing
    /// half animates the cloud and the petal scatter from this.
    #[must_use]
    pub fn boom_progress(&self) -> f32 {
        if self.expression == Expression::Boom {
            (self.expression_age / BOOM_SECONDS).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Audio-derived smoke level, `0..1`, before the `smoke` setting scales it.
    #[must_use]
    pub fn smoke(&self) -> f32 {
        self.smoke
    }

    /// Semantic warmth, `0..1`: a ~2 s follower over the Assist lane's valence,
    /// resting at [`NEUTRAL_WARMTH`] when the lane is absent or unconfident.
    /// The drawing half softens/warms the resting face and petal saturation
    /// from it, so two tracks at the same RMS read differently when the lane
    /// knows their mood.
    #[must_use]
    pub fn warmth(&self) -> f32 {
        self.warmth
    }

    /// Whether the last update saw an available, confident semantic lane. The
    /// report line prints it: a run with Assist data and `false` here means the
    /// lane never coupled.
    #[must_use]
    pub fn semantic_active(&self) -> bool {
        self.semantic_active
    }

    /// Sing-along mouth, `0..1`. The drawing half interpolates the resting `w`
    /// toward an `o` with it, so the character sings its own captions.
    #[must_use]
    pub fn mouth_open(&self) -> f32 {
        self.mouth_open
    }

    /// Eyelid position: `0` open, `1` closed, a triangular [`BLINK_SECONDS`]
    /// envelope. Held at `0` while the expression is [`Expression::Scrunch`] or
    /// [`Expression::Boom`] — those eyes are already special-cased strokes, and
    /// an eyelid over a scrunch would double-draw the same idea.
    #[must_use]
    pub fn blink(&self) -> f32 {
        if matches!(self.expression, Expression::Scrunch | Expression::Boom) {
            return 0.0;
        }
        if self.blink_age >= BLINK_SECONDS {
            return 0.0;
        }
        let half = BLINK_SECONDS * 0.5;
        if self.blink_age < half {
            self.blink_age / half
        } else {
            (BLINK_SECONDS - self.blink_age) / half
        }
    }

    /// Cats currently on the floor — the `cats` setting, clamped and rounded.
    #[must_use]
    pub fn cat_count(&self) -> usize {
        self.cat_count
    }

    /// The pose of one cat, or `None` past [`Self::cat_count`].
    #[must_use]
    pub fn cat(&self, index: usize) -> Option<CatPose> {
        if index >= self.cat_count {
            return None;
        }
        let count = self.cat_count.max(1) as f32;
        let base = (index as f32 + 0.5) / count;
        let wobble = (self.sway_phase * 0.37 + index as f32 * 2.4).sin() * 0.015;
        let lane = (base + self.cat_lane_jitter[index] * 0.05 + wobble).clamp(0.05, 0.95);
        let face = if index == self.curious_cat {
            CatFace::Curious
        } else if self.amplitude < 0.08 {
            CatFace::Sleepy
        } else if self.amplitude > 0.6 {
            CatFace::Excited
        } else {
            CatFace::Content
        };
        // The ballistic arc: zero before takeoff (a staggered cat is still
        // crouched), a half-sine through the air, exactly zero at and after
        // touchdown. The landing is a point in time, which is what an
        // exponential envelope never gave it.
        let age = self.hop_age[index];
        let hop = if age <= 0.0 || age >= HOP_SECONDS {
            0.0
        } else {
            (self.hop_power[index] * (std::f32::consts::PI * age / HOP_SECONDS).sin())
                .clamp(0.0, 1.0)
        };
        Some(CatPose {
            lane,
            hop,
            scale: self.cat_scale_jitter[index],
            flip: self.cat_flip[index],
            face,
        })
    }

    /// Bass transients heard so far. The report line prints it beside
    /// `beats=`: a track with an audible groove and `kicks=0` means the
    /// detector is deaf to this mix, which is a tuning fact, not a crash.
    #[must_use]
    pub fn kick_count(&self) -> u64 {
        self.kick_count
    }

    /// Strength of the last kick, `0.0` before the first.
    #[must_use]
    pub fn last_kick(&self) -> f32 {
        self.last_kick
    }

    /// How many characters of the current cue the terminal has typed.
    #[must_use]
    pub fn typed_chars(&self) -> usize {
        (self.typed as usize).min(self.typed_limit)
    }

    // -- the update helpers ----------------------------------------------------

    /// Instant attack, exponential release toward `target`, expressed as a
    /// per-second retention so preview and export agree at any frame rate.
    fn follow(previous: f32, target: f32, delta_seconds: f32, retention_per_second: f32) -> f32 {
        if !target.is_finite() {
            return previous;
        }
        if target >= previous {
            return target;
        }
        if delta_seconds <= 0.0 {
            return previous;
        }
        target + (previous - target) * retention_per_second.powf(delta_seconds)
    }

    /// Symmetric exponential approach — unlike [`Self::follow`], both
    /// directions move at the same rate. Warmth and the mouth want *gentle*,
    /// not fast-attack: an instant attack on valence would snap the palette on
    /// the first confident frame.
    fn approach(previous: f32, target: f32, delta_seconds: f32, retention_per_second: f32) -> f32 {
        if !target.is_finite() || delta_seconds <= 0.0 {
            return previous;
        }
        target + (previous - target) * retention_per_second.powf(delta_seconds)
    }

    fn clamp01(value: f32) -> f32 {
        value.clamp(0.0, 1.0)
    }

    fn raw_amplitude(audio: &SceneAudioFrame<'_>) -> f32 {
        Self::clamp01(audio.rms * 2.0)
    }

    fn raw_bass(audio: &SceneAudioFrame<'_>) -> f32 {
        Self::clamp01(bass_from_trails(audio.trails))
    }

    /// The bass-transient signal: mean positive excursion of the lowest
    /// quarter of the bands over their own trails. The same shape as
    /// [`SceneAudioFrame::spectral_flux`], scoped to the region
    /// [`bass_from_trails`] calls bass, so "the bass level" and "the bass
    /// moved" are measured over the same bands.
    fn raw_kick(audio: &SceneAudioFrame<'_>) -> f32 {
        let count = audio.bands.len().min(audio.trails.len());
        if count == 0 {
            return 0.0;
        }
        let low = (count / 4).max(1);
        audio.bands[..low]
            .iter()
            .zip(&audio.trails[..low])
            .map(|(band, trail)| (band - trail).max(0.0))
            .sum::<f32>()
            / low as f32
    }

    fn raw_flux(audio: &SceneAudioFrame<'_>) -> f32 {
        // The onset threshold is 0.08 on this quantity, so 6x maps a clear
        // onset to ~0.5 — the scale [`BUSY_FLUX`] is written against.
        Self::clamp01(audio.spectral_flux * 6.0)
    }

    /// Mean of petal `index`'s contiguous share of the analyzer bands.
    fn petal_target(bands: &[f32], index: usize, coupling: f32) -> f32 {
        if bands.is_empty() {
            return 0.0;
        }
        let start = index * bands.len() / PETAL_COUNT;
        let end = ((index + 1) * bands.len() / PETAL_COUNT).max(start + 1);
        let end = end.min(bands.len());
        if start >= end {
            return 0.0;
        }
        let mean: f32 = bands[start..end].iter().copied().sum::<f32>() / (end - start) as f32;
        Self::clamp01(mean * 1.7 * coupling)
    }

    fn set_expression(&mut self, next: Expression) {
        if self.expression != next {
            self.expression = next;
            self.expression_age = 0.0;
        }
    }

    /// The face's transition table. Priorities, top first: an active boom plays
    /// out untouchable; a new boom beats everything; pleading and dizzy are
    /// sustained conditions; a scrunch is an event; everything relaxes to happy.
    fn step_expression(&mut self, audio: &SceneAudioFrame<'_>, mood: f32, delta: f32) {
        self.expression_age += delta;
        self.boom_cooldown = (self.boom_cooldown - delta).max(0.0);

        if mood <= f32::EPSILON {
            // Pinned calm — a designed state, not a dead machine; the report
            // line still names the face so the two stay distinguishable.
            self.set_expression(Expression::Happy);
            return;
        }

        if self.expression == Expression::Boom {
            if self.expression_age >= BOOM_SECONDS {
                self.set_expression(Expression::Happy);
            }
            return;
        }

        // Entry thresholds scale down as mood scales up. The clamps keep a
        // maximal mood from making every frame an event and a minimal one from
        // needing physically impossible input.
        // The bass gate reads [`bass_from_trails`] — the mean of the lowest
        // *quarter* of the smoothed bands — which tops out well under 1.0 on
        // real material because the quarter spans more than the kick. 0.6 at
        // neutral mood is "the low end is genuinely full", measured against the
        // synthetic fixtures; 0.72 was never reached at all.
        let boom_amplitude = (0.88 / mood).clamp(0.70, 0.98);
        let boom_bass = (0.60 / mood).clamp(0.45, 0.90);
        let scrunch_flux = (0.11 / mood).clamp(0.06, 0.40);
        let plead_after = (3.0 / mood).clamp(1.5, 10.0);
        let dizzy_after = (2.8 / mood).clamp(1.2, 8.0);

        if audio.onset
            && self.amplitude >= boom_amplitude
            && self.bass >= boom_bass
            && self.boom_cooldown <= 0.0
        {
            self.set_expression(Expression::Boom);
            self.boom_cooldown = BOOM_COOLDOWN_SECONDS;
            return;
        }
        if self.quiet_seconds >= plead_after {
            self.set_expression(Expression::Pleading);
            return;
        }
        if self.busy_seconds >= dizzy_after {
            self.set_expression(Expression::Dizzy);
            return;
        }
        if audio.onset && audio.spectral_flux >= scrunch_flux {
            self.set_expression(Expression::Scrunch);
            return;
        }

        match self.expression {
            Expression::Scrunch if self.expression_age >= SCRUNCH_SECONDS => {
                self.set_expression(Expression::Happy);
            }
            Expression::Pleading if self.amplitude > RECOVER_AMPLITUDE => {
                self.set_expression(Expression::Happy);
            }
            Expression::Dizzy if self.flux < CALM_FLUX => {
                self.set_expression(Expression::Happy);
            }
            _ => {}
        }
    }
}

impl SceneState for ClawdState {
    fn id(&self) -> SceneId {
        SceneId::Clawd
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        let delta = if frame.delta_seconds.is_finite() && frame.delta_seconds > 0.0 {
            frame.delta_seconds.min(MAX_STEP_SECONDS)
        } else {
            0.0
        };
        let audio = &frame.audio;

        // Envelopes. The un-capped delta is fine here — a longer stall only
        // releases further, which is the truthful reading of a seek.
        self.amplitude = Self::follow(
            self.amplitude,
            Self::raw_amplitude(audio),
            frame.delta_seconds,
            ENVELOPE_RELEASE_PER_SECOND,
        );
        self.bass = Self::follow(
            self.bass,
            Self::raw_bass(audio),
            frame.delta_seconds,
            ENVELOPE_RELEASE_PER_SECOND,
        );
        self.flux = Self::follow(
            self.flux,
            Self::raw_flux(audio),
            frame.delta_seconds,
            ENVELOPE_RELEASE_PER_SECOND,
        );
        let coupling = frame.setting(SceneId::Clawd, setting::PETALS);
        for index in 0..PETAL_COUNT {
            self.petals[index] = Self::follow(
                self.petals[index],
                Self::petal_target(audio.bands, index, coupling),
                frame.delta_seconds,
                PETAL_RELEASE_PER_SECOND,
            );
        }

        // Motion clocks.
        let spin = frame.setting(SceneId::Clawd, setting::SPIN);
        self.petal_angle += delta * spin * (0.45 + 0.55 * self.amplitude);
        self.sway_phase += delta * (0.9 + 0.8 * self.amplitude);

        // The cat count resolves before anything reads it. It used to resolve
        // at the end of the update, which left the kick block one frame stale —
        // harmless every frame except the first, where a transient on frame
        // zero met a count of 0 and hopped nobody.
        let cats = frame.setting(SceneId::Clawd, setting::CATS);
        self.cat_count = (cats.round().max(0.0) as usize).min(MAX_CATS);

        // The bass transient: what the choreography actually dances to. See
        // [`KICK_THRESHOLD`] for why this replaced the tracker's phase wraps —
        // the cat_probe run on real material is the whole argument.
        let kick = Self::raw_kick(audio);
        self.kick_refractory = (self.kick_refractory - delta).max(0.0);
        if kick >= KICK_THRESHOLD && self.kick_refractory <= 0.0 {
            self.kick_refractory = KICK_REFRACTORY_SECONDS;
            self.kick_count = self.kick_count.wrapping_add(1);
            self.last_kick = kick;
            self.bounce = 1.0;
            if self.cat_count > 0 {
                // Height follows how hard the track hit, floored so a hop that
                // fires is a hop a viewer can see.
                let power = (kick / (2.0 * KICK_THRESHOLD)).clamp(0.55, 1.0);
                if kick >= PARTY_KICK && self.amplitude >= PARTY_AMPLITUDE {
                    // A genuinely big moment: everybody up, each on their own
                    // seeded stagger so the row reads as six cats, not one.
                    for index in 0..MAX_CATS {
                        self.hop_age[index] = -self.hop_stagger[index];
                        self.hop_power[index] = power;
                    }
                } else {
                    // Cats take turns rather than pogoing in unison.
                    let hopper = (self.kick_count % self.cat_count as u64) as usize;
                    self.hop_age[hopper] = -self.hop_stagger[hopper];
                    self.hop_power[hopper] = power;
                }
            }
        }

        // The tracker. `beat_phase` saws 0→1 per beat; a wrap is a beat as the
        // tracker believes it. Worth a soft nod of the head and the count the
        // report prints — never a cat (see [`WRAP_BOUNCE`]).
        let phase = audio.beat_phase;
        if phase.is_finite() && phase < self.previous_beat_phase - 0.5 {
            self.beat_count = self.beat_count.wrapping_add(1);
            self.bounce = self.bounce.max(WRAP_BOUNCE);
        }
        self.previous_beat_phase = if phase.is_finite() { phase } else { 0.0 };
        self.bounce = Self::follow(self.bounce, 0.0, delta, BOUNCE_RELEASE_PER_SECOND);
        for age in &mut self.hop_age {
            if *age < HOP_SECONDS {
                *age += delta;
            }
        }

        // Sustained-condition clocks for the face.
        if self.amplitude < QUIET_AMPLITUDE {
            self.quiet_seconds += delta;
        } else {
            self.quiet_seconds = 0.0;
        }
        if self.flux > BUSY_FLUX {
            self.busy_seconds += delta;
        } else {
            self.busy_seconds = (self.busy_seconds - delta * 2.0).max(0.0);
        }
        // The semantic lane. Warmth follows valence only when the lane clears
        // the confidence floor; an unconfident interpretation is review
        // material, not something to paint the character with.
        let semantic = &frame.semantic;
        self.semantic_active =
            semantic.available && semantic.confidence >= SEMANTIC_CONFIDENCE_FLOOR;
        let warmth_target = if self.semantic_active {
            Self::clamp01(semantic.valence)
        } else {
            NEUTRAL_WARMTH
        };
        self.warmth = Self::approach(
            self.warmth,
            warmth_target,
            delta,
            WARMTH_RETENTION_PER_SECOND,
        );

        // Tension biases the expression thresholds: a tense track should
        // scrunch easier at the same slider setting. Weighted by confidence
        // (an unsure "tense" barely leans) and capped at the mood slider's own
        // ceiling, so the bias can never push the thresholds past what the
        // threshold clamps were written against. Mood 0 stays pinned calm:
        // zero times any bias is zero.
        let mood = frame.setting(SceneId::Clawd, setting::MOOD);
        let mood = if semantic.available {
            let bias = 1.0
                + TENSION_MOOD_GAIN
                    * semantic.tension.clamp(0.0, 1.0)
                    * semantic.confidence.clamp(0.0, 1.0);
            (mood * bias).min(MOOD_CEILING)
        } else {
            mood
        };
        self.step_expression(audio, mood, delta);

        // Blinks. The countdown runs unconditionally — through scrunches and
        // booms too — but only starts an eyelid when the face can take one;
        // a suppressed expiry just re-draws the interval and moves on.
        self.blink_countdown -= delta;
        if self.blink_countdown <= 0.0 {
            self.blink_countdown = self.blink_rng.next_range(
                f64::from(BLINK_INTERVAL_MIN_SECONDS),
                f64::from(BLINK_INTERVAL_MAX_SECONDS),
            ) as f32;
            if !matches!(self.expression, Expression::Scrunch | Expression::Boom) {
                self.blink_age = 0.0;
            }
        }
        if self.blink_age < BLINK_SECONDS {
            self.blink_age += delta;
        }

        // Smoke: rises when the track sustains, thins slowly when it stops.
        // Asymmetric on purpose — smoke that vanished on the first quiet bar
        // would read as a rendering fault, not weather.
        let smoke_target = Self::clamp01((self.amplitude - 0.22) * 1.5);
        let remain = if smoke_target > self.smoke {
            0.15f32
        } else {
            0.75f32
        };
        self.smoke += (smoke_target - self.smoke) * (1.0 - remain.powf(delta));

        // Cats.

        // The lyric terminal's typist. Runs regardless of the toggle — see the
        // module docs for why the toggle gates drawing, not time.
        match frame.lyric {
            Some(cue) => {
                if self.typed_cue != Some(cue.id) {
                    self.typed_cue = Some(cue.id);
                    self.typed = 0.0;
                    self.typed_limit = cue.text.chars().count();
                }
                self.typed =
                    (self.typed + delta * TYPE_CHARS_PER_SECOND).min(self.typed_limit as f32 + 1.0);
            }
            None => {
                self.typed_cue = None;
                self.typed = 0.0;
                self.typed_limit = 0;
            }
        }

        // The sing-along mouth, after the typist so `typed_cue` reflects this
        // frame. Flux sets how far the mouth can open; the syllable-rate
        // carrier flaps it; the fast follower keeps the flap from aliasing.
        // The carrier only advances while singing — a phase that ran between
        // cues would reopen each line at an arbitrary point of the flap.
        let singing = frame.lyric.is_some() && self.typed_cue.is_some();
        if singing {
            self.sing_phase +=
                delta * SING_RATE_HZ * std::f32::consts::TAU * (0.6 + 0.8 * self.amplitude);
            let carrier = 0.5 + 0.5 * self.sing_phase.sin();
            let target = Self::clamp01(self.flux * MOUTH_FLUX_GAIN) * carrier;
            self.mouth_open = Self::approach(
                self.mouth_open,
                target,
                delta,
                MOUTH_ATTACK_RETENTION_PER_SECOND,
            );
        } else {
            self.mouth_open = Self::approach(
                self.mouth_open,
                0.0,
                delta,
                MOUTH_RELEASE_RETENTION_PER_SECOND,
            );
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
        id: SceneId::Clawd,
        state_version: STATE_VERSION,
        make_state: |seed| Box::new(ClawdState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::{descriptor as setting_descriptor, SceneSettings};
    use crate::scene::{LyricCue, SemanticFrame};

    const SCENE: SceneId = SceneId::Clawd;

    /// An available semantic lane, in the fixture-helper style above.
    fn semantic(valence: f32, tension: f32, confidence: f32) -> SemanticFrame {
        SemanticFrame {
            available: true,
            source_id: 1,
            energy: 0.5,
            tension,
            valence,
            confidence,
        }
    }

    fn frame_with<'a>(
        settings: &'a SceneSettings,
        bands: &'a [f32],
        trails: &'a [f32],
        delta: f32,
    ) -> SceneFrame<'a> {
        let mut frame = SceneFrame::idle(settings);
        frame.delta_seconds = delta;
        frame.audio = SceneAudioFrame::from_spectrum(bands, trails);
        frame
    }

    /// Enough loud frames to drive the amplitude envelope up.
    fn feed_loud(state: &mut ClawdState, settings: &SceneSettings, frames: usize) {
        let bands = [0.9f32; 24];
        let trails = [0.85f32; 24];
        for _ in 0..frames {
            let frame = frame_with(settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
    }

    fn feed_silence(state: &mut ClawdState, settings: &SceneSettings, frames: usize) {
        let bands = [0.0f32; 24];
        let trails = [0.0f32; 24];
        for _ in 0..frames {
            let frame = frame_with(settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
    }

    #[test]
    fn the_anatomy_and_the_descriptor_bounds_agree() {
        // The `cats` descriptor maximum is the pose-table size; a descriptor
        // that grew without the table growing would clamp silently.
        let cats = setting_descriptor(SCENE, setting::CATS).expect("cats descriptor");
        assert_eq!(cats.maximum as usize, MAX_CATS);
        // The tension bias caps effective mood at the slider's own maximum; if
        // the descriptor moved without this constant, the bias could push the
        // thresholds past what their clamps were written against.
        let mood = setting_descriptor(SCENE, setting::MOOD).expect("mood descriptor");
        assert_eq!(mood.maximum, MOOD_CEILING);
        assert_eq!(crate::scene::SCENE_COUNT, 12);
        // Preset-store token derivation reads descriptor 0's key segment; it
        // must equal the stable name or presets reload onto the wrong scene.
        let first = setting_descriptor(SCENE, 0).expect("first descriptor");
        assert!(first.key.starts_with("settings.clawd."));
        assert_eq!(SCENE.stable_name(), "clawd");
    }

    #[test]
    fn the_registry_descriptor_builds_a_downcastable_state() {
        let descriptor = descriptor();
        assert_eq!(descriptor.id, SCENE);
        let state = (descriptor.make_state)(7);
        assert_eq!(state.id(), SCENE);
        assert!(state.as_any().downcast_ref::<ClawdState>().is_some());
    }

    #[test]
    fn the_same_seed_and_frames_reproduce_the_same_state() {
        let settings = SceneSettings::new();
        let mut a = ClawdState::new(42);
        let mut b = ClawdState::new(42);
        let bands = [
            0.1f32, 0.9, 0.2, 0.8, 0.3, 0.7, 0.4, 0.6, 0.5, 0.5, 0.6, 0.4,
        ];
        let trails = [0.05f32; 12];
        // A cue and a semantic lane so the senses wave has state to diverge in
        // if it ever picked up an unseeded input.
        let cue = LyricCue {
            id: 3,
            start_seconds: 0.0,
            end_seconds: 8.0,
            text: "same seed, same eyelids".to_string(),
        };
        for i in 0..240 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.audio.beat_phase = (i as f32 * 0.02) % 1.0;
            frame.semantic = semantic(0.8, 0.5, 0.9);
            frame.lyric = Some(&cue);
            a.update(&frame);
            b.update(&frame);
        }
        assert_eq!(a.petals(), b.petals());
        assert_eq!(a.expression(), b.expression());
        assert_eq!(a.bounce(), b.bounce());
        assert_eq!(a.beat_count(), b.beat_count());
        assert_eq!(a.warmth(), b.warmth());
        assert_eq!(a.mouth_open(), b.mouth_open());
        assert_eq!(a.blink(), b.blink());
        for i in 0..a.cat_count() {
            assert_eq!(a.cat(i), b.cat(i));
        }
    }

    #[test]
    fn petals_track_their_own_band_group() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(1);
        // Energy only in the tenth twelfth of the spectrum.
        let mut bands = [0.0f32; 48];
        for band in &mut bands[36..40] {
            *band = 1.0;
        }
        let trails = [0.0f32; 48];
        for _ in 0..30 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert!(
            state.petals()[9] > 0.8,
            "petal 9 should carry its band group's energy, got {}",
            state.petals()[9]
        );
        assert!(
            state.petals()[0] < 0.05,
            "petal 0 had no energy in its group, got {}",
            state.petals()[0]
        );
        // Coupling at zero flattens every petal.
        let mut flat = SceneSettings::new();
        assert!(flat.set(SCENE, setting::PETALS, 0.0));
        let mut state = ClawdState::new(1);
        for _ in 0..30 {
            let frame = frame_with(&flat, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert!(state.petals().iter().all(|&p| p == 0.0));
    }

    #[test]
    fn a_hard_onset_scrunches_and_the_face_relaxes_back() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(3);
        // Moderate energy so neither pleading nor boom is in reach.
        let bands = [0.35f32; 24];
        let trails = [0.34f32; 24];
        for _ in 0..30 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Happy);
        // One frame of strong flux: bands well above their trails.
        let spike = [0.6f32; 24];
        let low_trails = [0.3f32; 24];
        let frame = frame_with(&settings, &spike, &low_trails, 1.0 / 60.0);
        assert!(frame.audio.onset, "the fixture must actually onset");
        state.update(&frame);
        assert_eq!(state.expression(), Expression::Scrunch);
        // It holds for its dwell, then relaxes.
        for _ in 0..30 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Happy);
    }

    #[test]
    fn long_quiet_pleads_and_energy_recovers_it() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(4);
        feed_silence(&mut state, &settings, 60 * 4);
        assert_eq!(state.expression(), Expression::Pleading);
        feed_loud(&mut state, &settings, 30);
        assert_eq!(state.expression(), Expression::Happy);
    }

    #[test]
    fn sustained_flux_dizzies_and_calm_recovers_it() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(5);
        // Bands persistently far above their trails: continuous high flux.
        let bands = [0.5f32; 24];
        let trails = [0.05f32; 24];
        for _ in 0..60 * 4 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Dizzy);
        // Settle: bands equal trails, flux dies, the spell ends.
        let settled = [0.5f32; 24];
        for _ in 0..60 * 6 {
            let frame = frame_with(&settings, &settled, &settled, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Happy);
    }

    #[test]
    fn the_boom_fires_once_and_respects_its_cooldown() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(6);
        // Saturate every envelope, then onset. Trails at 0.7 keep the flux well
        // above the onset threshold while `bass_from_trails` reads 0.7 — past
        // the 0.6 boom gate but not an unphysical 1.0.
        let bands = [1.0f32; 24];
        let low_trails = [0.7f32; 24];
        for _ in 0..60 {
            let frame = frame_with(&settings, &bands, &low_trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Boom);
        // A second peak right after the playout must not boom again.
        for _ in 0..(BOOM_SECONDS * 60.0) as usize + 10 {
            let frame = frame_with(&settings, &bands, &low_trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_ne!(state.expression(), Expression::Boom);
        // After the cooldown, it may.
        let quiet = [0.0f32; 24];
        for _ in 0..(BOOM_COOLDOWN_SECONDS * 4.0) as usize {
            let frame = frame_with(&settings, &quiet, &quiet, 0.25);
            state.update(&frame);
        }
        for _ in 0..60 {
            let frame = frame_with(&settings, &bands, &low_trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Boom);
    }

    #[test]
    fn mood_zero_pins_the_resting_smile() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SCENE, setting::MOOD, 0.0));
        let mut state = ClawdState::new(7);
        let bands = [1.0f32; 24];
        let low_trails = [0.2f32; 24];
        for _ in 0..240 {
            let frame = frame_with(&settings, &bands, &low_trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.expression(), Expression::Happy);
    }

    #[test]
    fn a_beat_wrap_soft_bobs_the_head_and_never_hops_a_cat() {
        // Bands equal to trails: no bass transient anywhere in this fixture,
        // so anything that moves moved on the tracker's say-so alone — and the
        // tracker freewheels between anchors on real material (cat_probe,
        // 2026-08-25), which is why its wraps are worth a nod and not a jump.
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(8);
        let bands = [0.5f32; 24];
        let trails = [0.5f32; 24];
        let mut bobbed = false;
        for i in 0..120 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            // A 1-second beat sawtooth.
            frame.audio.beat_phase = (i as f32 / 60.0) % 1.0;
            state.update(&frame);
            if state.bounce() > 0.3 {
                bobbed = true;
            }
            assert!(
                state.bounce() <= 0.5,
                "a wrap is a soft bob, not the kick's full impulse: {}",
                state.bounce()
            );
            let hopping =
                (0..state.cat_count()).any(|c| state.cat(c).is_some_and(|cat| cat.hop > 0.0));
            assert!(!hopping, "a freewheeling wrap must never hop a cat");
        }
        assert!(bobbed, "a phase wrap should still nod the head");
        assert_eq!(state.beat_count(), 1);
        assert_eq!(state.kick_count(), 0);
    }

    #[test]
    fn a_bass_transient_hops_one_cat_on_a_ballistic_arc() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(8);
        // Moderate kick: lowest quarter well above its trails, quiet enough
        // overall that the party rule stays out of it.
        let mut bands = [0.25f32; 24];
        for band in &mut bands[..6] {
            *band = 0.45;
        }
        let trails = [0.25f32; 24];
        // One transient frame, then settled audio (bands == trails).
        let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
        state.update(&frame);
        assert_eq!(state.kick_count(), 1);
        assert!(state.last_kick() > 0.0);
        assert!(
            (state.bounce() - 1.0).abs() < 0.2,
            "a kick is the full impulse"
        );
        let settled = [0.25f32; 24];
        let mut peak = 0.0f32;
        let mut airborne_frames = 0u32;
        for _ in 0..60 {
            let frame = frame_with(&settings, &settled, &settled, 1.0 / 60.0);
            state.update(&frame);
            let hop = (0..state.cat_count())
                .filter_map(|c| state.cat(c))
                .map(|cat| cat.hop)
                .fold(0.0f32, f32::max);
            if hop > 0.0 {
                airborne_frames += 1;
            }
            peak = peak.max(hop);
        }
        assert!(peak >= 0.5, "the arc must reach a visible apex, got {peak}");
        // Ballistic, not asymptotic: with stagger the flight is bounded well
        // under half a second, and it *ends* — the landing is a frame, not a
        // limit.
        assert!(
            airborne_frames > 0 && (airborne_frames as f32) < 30.0,
            "the hop should be airborne briefly and then land: {airborne_frames} frames"
        );
        let grounded =
            (0..state.cat_count()).all(|c| state.cat(c).is_some_and(|cat| cat.hop == 0.0));
        assert!(grounded, "every cat is exactly on the floor after landing");
    }

    #[test]
    fn a_sustained_excursion_is_metered_by_the_refractory() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(8);
        // Two seconds of continuous strong bass excursion.
        let mut bands = [0.3f32; 24];
        for band in &mut bands[..6] {
            *band = 0.8;
        }
        let trails = [0.3f32; 24];
        for _ in 0..120 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        // 2 s / 0.25 s refractory = at most 8, and at least half that — a
        // drumroll is pushes, not a blur.
        let kicks = state.kick_count();
        assert!((4..=8).contains(&kicks), "got {kicks} kicks in 2 s");
    }

    #[test]
    fn a_big_kick_on_a_loud_track_parties_every_cat_with_stagger() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SCENE, setting::CATS, 6.0));
        let mut state = ClawdState::new(8);
        // Saturate the amplitude envelope first (bands == trails: no kick).
        let loud = [0.9f32; 24];
        for _ in 0..30 {
            let frame = frame_with(&settings, &loud, &loud, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.kick_count(), 0, "settled loudness alone is no kick");
        // Now a heavy bass transient.
        let mut bands = [0.9f32; 24];
        for band in &mut bands[..6] {
            *band = 1.0;
        }
        let trails = [0.55f32; 24];
        let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
        state.update(&frame);
        assert_eq!(state.kick_count(), 1);
        // Every cat takes off, each on its own seeded stagger: within a few
        // frames all six are airborne, but not from the identical instant.
        let settled = [0.9f32; 24];
        let mut all_up_at_once = false;
        for _ in 0..12 {
            let frame = frame_with(&settings, &settled, &settled, 1.0 / 60.0);
            state.update(&frame);
            let up = (0..MAX_CATS)
                .filter(|&c| state.cat(c).is_some_and(|cat| cat.hop > 0.0))
                .count();
            if up == MAX_CATS {
                all_up_at_once = true;
            }
        }
        assert!(all_up_at_once, "a party should get every cat airborne");
        let staggers_differ =
            (1..MAX_CATS).any(|i| (state.hop_stagger[i] - state.hop_stagger[0]).abs() > 1.0e-4);
        assert!(staggers_differ, "seeded staggers must not be uniform");
    }

    #[test]
    fn cats_follow_their_setting_and_stay_in_bounds() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SCENE, setting::CATS, 6.0));
        let mut state = ClawdState::new(9);
        feed_loud(&mut state, &settings, 10);
        assert_eq!(state.cat_count(), MAX_CATS);
        for i in 0..MAX_CATS {
            let cat = state.cat(i).expect("a cat inside the count");
            assert!((0.0..=1.0).contains(&cat.lane), "lane {}", cat.lane);
            assert!((0.0..=1.0).contains(&cat.hop), "hop {}", cat.hop);
            assert!((0.5..=1.5).contains(&cat.scale), "scale {}", cat.scale);
        }
        assert!(state.cat(MAX_CATS).is_none());
        assert!(settings.set(SCENE, setting::CATS, 0.0));
        feed_loud(&mut state, &settings, 2);
        assert_eq!(state.cat_count(), 0);
        assert!(state.cat(0).is_none());
        // Exactly one curious cat, and it is stable across frames.
        assert!(settings.set(SCENE, setting::CATS, 6.0));
        feed_loud(&mut state, &settings, 2);
        let curious: Vec<usize> = (0..MAX_CATS)
            .filter(|&i| state.cat(i).is_some_and(|c| c.face == CatFace::Curious))
            .collect();
        assert_eq!(curious.len(), 1);
        let who = curious[0];
        feed_loud(&mut state, &settings, 30);
        assert!(state.cat(who).is_some_and(|c| c.face == CatFace::Curious));
    }

    #[test]
    fn the_typist_types_resets_on_a_new_cue_and_clears_without_one() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(10);
        let cue = LyricCue {
            id: 5,
            start_seconds: 0.0,
            end_seconds: 4.0,
            text: "bucket! bucket!".to_string(),
        };
        let bands = [0.2f32; 8];
        let trails = [0.2f32; 8];
        for _ in 0..12 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.lyric = Some(&cue);
            state.update(&frame);
        }
        // 12 frames at 28 cps ≈ 5.6 chars.
        let partial = state.typed_chars();
        assert!(partial > 0 && partial < cue.text.chars().count());
        for _ in 0..120 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.lyric = Some(&cue);
            state.update(&frame);
        }
        assert_eq!(state.typed_chars(), cue.text.chars().count());
        // A new cue restarts the typist.
        let next = LyricCue {
            id: 6,
            text: "nearest wall and call it home".to_string(),
            ..cue.clone()
        };
        let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
        frame.lyric = Some(&next);
        state.update(&frame);
        assert!(state.typed_chars() < 2);
        // No cue clears it.
        let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
        state.update(&frame);
        assert_eq!(state.typed_chars(), 0);
    }

    #[test]
    fn smoke_rises_under_sustained_energy_and_thins_slower_than_it_rose() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(11);
        feed_loud(&mut state, &settings, 60 * 3);
        let risen = state.smoke();
        assert!(
            risen > 0.5,
            "three loud seconds should build smoke, got {risen}"
        );
        feed_silence(&mut state, &settings, 60);
        assert!(
            state.smoke() > risen * 0.4,
            "one quiet second must not clear the smoke: {} -> {}",
            risen,
            state.smoke()
        );
        feed_silence(&mut state, &settings, 60 * 30);
        assert!(
            state.smoke() < 0.05,
            "smoke eventually clears, got {}",
            state.smoke()
        );
    }

    #[test]
    fn a_stalled_frame_advances_the_character_by_at_most_the_cap() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(12);
        let cue = LyricCue {
            id: 1,
            start_seconds: 0.0,
            end_seconds: 60.0,
            text: "the distribution has an edge".to_string(),
        };
        let bands = [0.2f32; 8];
        let trails = [0.2f32; 8];
        let mut frame = frame_with(&settings, &bands, &trails, 10.0);
        frame.lyric = Some(&cue);
        state.update(&frame);
        // 10 s at 28 cps would be the whole line; the cap admits 7 chars.
        assert!(
            state.typed_chars() <= (MAX_STEP_SECONDS * TYPE_CHARS_PER_SECOND) as usize,
            "a seek must not teleport the typist: {}",
            state.typed_chars()
        );
    }

    #[test]
    fn spin_follows_its_setting_and_its_sign() {
        let mut settings = SceneSettings::new();
        assert!(settings.set(SCENE, setting::SPIN, 2.0));
        let mut state = ClawdState::new(13);
        feed_loud(&mut state, &settings, 60);
        assert!(state.petal_angle() > 0.3);
        assert!(settings.set(SCENE, setting::SPIN, -2.0));
        let mut state = ClawdState::new(13);
        feed_loud(&mut state, &settings, 60);
        assert!(state.petal_angle() < -0.3);
        assert!(settings.set(SCENE, setting::SPIN, 0.0));
        let mut state = ClawdState::new(13);
        feed_loud(&mut state, &settings, 60);
        assert_eq!(state.petal_angle(), 0.0);
    }

    #[test]
    fn warmth_follows_a_confident_lane_and_holds_neutral_otherwise() {
        let settings = SceneSettings::new();
        let bands = [0.3f32; 24];
        let trails = [0.3f32; 24];

        // A confident, warm lane: four seconds is two time constants, so the
        // follower has covered ~86 % of the distance from 0.6 to 0.9.
        let mut state = ClawdState::new(14);
        for _ in 0..240 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.semantic = semantic(0.9, 0.0, 0.9);
            state.update(&frame);
        }
        assert!(state.semantic_active());
        assert!(
            state.warmth() > 0.8 && state.warmth() < 0.9,
            "warmth should be most of the way to valence 0.9, got {}",
            state.warmth()
        );

        // No lane at all: warmth never leaves neutral. Exact equality holds
        // because the follower's target equals its state.
        let mut state = ClawdState::new(14);
        for _ in 0..240 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert!(!state.semantic_active());
        assert_eq!(state.warmth(), NEUTRAL_WARMTH);

        // An available but unconfident lane is treated as absent: 0.2 is under
        // the 0.35 floor, so the valence 0.9 must not pull warmth at all.
        let mut state = ClawdState::new(14);
        for _ in 0..240 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.semantic = semantic(0.9, 0.0, 0.2);
            state.update(&frame);
        }
        assert!(!state.semantic_active());
        assert_eq!(state.warmth(), NEUTRAL_WARMTH);
    }

    #[test]
    fn tension_lowers_the_scrunch_bar_at_the_same_mood() {
        // Flux 0.095 clears the 0.08 onset threshold but not the neutral-mood
        // 0.11 scrunch bar; at tension 1.0 / confidence 1.0 the effective mood
        // is 1.6 and the bar drops to ~0.069, which 0.095 does clear. Levels
        // stay low (amplitude 0.6, bass ~0.3) so neither boom gate is in play.
        let settings = SceneSettings::new();
        let calm_bands = [0.3f32; 24];
        let calm_trails = [0.3f32; 24];
        let spike_bands = [0.3f32; 24];
        let spike_trails = [0.205f32; 24];

        let run = |tension: f32| {
            let mut state = ClawdState::new(15);
            for _ in 0..30 {
                let mut frame = frame_with(&settings, &calm_bands, &calm_trails, 1.0 / 60.0);
                frame.semantic = semantic(0.5, tension, 1.0);
                state.update(&frame);
            }
            let mut frame = frame_with(&settings, &spike_bands, &spike_trails, 1.0 / 60.0);
            assert!(frame.audio.onset, "the fixture must actually onset");
            frame.semantic = semantic(0.5, tension, 1.0);
            state.update(&frame);
            state.expression()
        };

        assert_eq!(
            run(0.0),
            Expression::Happy,
            "at tension 0 this onset is under the scrunch bar"
        );
        assert_eq!(
            run(1.0),
            Expression::Scrunch,
            "at full tension the same onset must scrunch"
        );
    }

    #[test]
    fn the_mouth_sings_during_a_cue_and_closes_without_one() {
        let settings = SceneSettings::new();
        let mut state = ClawdState::new(16);
        let cue = LyricCue {
            id: 9,
            start_seconds: 0.0,
            end_seconds: 8.0,
            text: "open wide and mean it".to_string(),
        };
        // Strong flux: bands well above trails, so the flux envelope saturates
        // and the carrier is what shapes the mouth.
        let bands = [0.5f32; 24];
        let trails = [0.05f32; 24];
        let mut widest = 0.0f32;
        for _ in 0..120 {
            let mut frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            frame.lyric = Some(&cue);
            state.update(&frame);
            widest = widest.max(state.mouth_open());
        }
        assert!(
            widest > 0.4,
            "two seconds of cue and flux should open the mouth, got {widest}"
        );

        // The cue ends: ~150 ms release, so one second closes it entirely.
        let calm = [0.3f32; 24];
        for _ in 0..60 {
            let frame = frame_with(&settings, &calm, &calm, 1.0 / 60.0);
            state.update(&frame);
        }
        assert!(
            state.mouth_open() < 0.02,
            "no cue must close the mouth, got {}",
            state.mouth_open()
        );

        // Flux with no cue never opens it in the first place: the release
        // limb's target is 0, and 0 toward 0 stays exactly 0.
        let mut state = ClawdState::new(16);
        for _ in 0..120 {
            let frame = frame_with(&settings, &bands, &trails, 1.0 / 60.0);
            state.update(&frame);
        }
        assert_eq!(state.mouth_open(), 0.0);
    }

    #[test]
    fn blinks_happen_are_seeded_and_never_cross_a_scrunch_or_boom() {
        let settings = SceneSettings::new();
        // Calm frames: the face stays Happy, so nothing suppresses. The first
        // countdown is at most 6.5 s, so ten seconds must contain a blink, and
        // the 1/60 s sampling lands within 0.008 s of the triangular peak.
        let calm = [0.3f32; 24];
        let mut a = ClawdState::new(21);
        let mut b = ClawdState::new(21);
        let mut deepest = 0.0f32;
        for _ in 0..600 {
            let frame = frame_with(&settings, &calm, &calm, 1.0 / 60.0);
            a.update(&frame);
            b.update(&frame);
            assert_eq!(a.blink(), b.blink(), "same seed, same eyelids");
            deepest = deepest.max(a.blink());
        }
        assert!(
            deepest > 0.5,
            "ten calm seconds must contain a blink, deepest {deepest}"
        );

        // Periodic hard onsets scrunch the face; whenever it is scrunched (or
        // boomed) the eyelid must hold 0 even if the schedule fires under it.
        let spike_bands = [0.5f32; 24];
        let spike_trails = [0.2f32; 24];
        let mut state = ClawdState::new(22);
        let mut scrunched = false;
        for i in 0..720 {
            let frame = if i % 30 == 0 {
                frame_with(&settings, &spike_bands, &spike_trails, 1.0 / 60.0)
            } else {
                frame_with(&settings, &calm, &calm, 1.0 / 60.0)
            };
            state.update(&frame);
            if matches!(state.expression(), Expression::Scrunch | Expression::Boom) {
                scrunched = true;
                assert_eq!(
                    state.blink(),
                    0.0,
                    "a scrunched or boomed face never also blinks"
                );
            }
        }
        assert!(scrunched, "the fixture must actually scrunch");
    }
}
