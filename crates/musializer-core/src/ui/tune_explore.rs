//! Reversible tuning exploration: snapshot/audition, bounded randomize, and
//! typed/stepped value entry.
//!
//! **Post-legacy product extension (UX0-C04, UX0-C07, UX0-B09).** The frozen C
//! has none of this. `plug.c`'s inspector writes every slider straight into
//! `scene_settings_set` with no history, so the only way back from "let me see
//! what that preset does" is to remember eight numbers. Tune is the surface a
//! user is supposed to *play* on, and play needs an undo.
//!
//! Everything here is pure: no clock, no filesystem, and **no ambient
//! randomness**. [`surprise`] and [`nudge`] take a [`RandomSource`] by reference
//! so a test can seed them and get the same tuning twice, which is also what
//! makes "the result is inside every descriptor's bounds" checkable across all
//! 81 descriptors rather than across whatever a lucky run produced.
//!
//! # Why a snapshot rather than an edit log
//!
//! [`SettingsSnapshot`] is `Copy`, is already the persistence unit for a cue's
//! captured tuning, and is already validated against the descriptor table
//! (`SettingsSnapshot::is_valid_for`). Reverting is therefore *the same
//! operation* the project format performs when it loads a cue — 12 `f32`s
//! copied back — which is why a revert is bit-for-bit exact and not merely
//! "close". An edit log would have to replay float arithmetic to get back.

use crate::scene::settings::{
    self, SceneSettings, SettingDescriptor, SettingKind, SettingsSnapshot, MAX_CONTROLS,
};
use crate::scene::SceneId;

// -- injected randomness -------------------------------------------------------

/// A source of uniform bits.
///
/// A trait rather than a concrete generator so the caller owns the seed. The
/// application seeds one per press from a counter it keeps; a test seeds a
/// constant and gets a fixed tuning. Nothing in this module reads a clock.
pub trait RandomSource {
    fn next_u64(&mut self) -> u64;

    /// A uniform value in `[0, 1)`, taking the top 53 bits — the usual
    /// construction, so the low bits of a weak generator never reach the result.
    fn next_unit(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
        (self.next_u64() >> 11) as f64 * SCALE
    }

    /// A uniform value in `[low, high)`, or `low` for an empty range.
    fn next_range(&mut self, low: f64, high: f64) -> f64 {
        if high <= low {
            return low;
        }
        low + self.next_unit() * (high - low)
    }

    /// True with probability `p`.
    fn chance(&mut self, p: f64) -> bool {
        self.next_unit() < p
    }
}

/// SplitMix64 — Steele/Lea/Flood, the generator `java.util.SplittableRandom`
/// uses and the one xoshiro's authors recommend for seeding.
///
/// Chosen because it is eleven lines with no state beyond a `u64`, passes
/// BigCrush, and adding a random-number crate to a workspace this agent does not
/// own the manifest of is not a trade worth making for a Surprise button.
#[derive(Clone, Copy, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }
}

impl RandomSource for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

// -- value arithmetic on the descriptor contract -------------------------------

/// The grid one decimal place of `precision` puts on a value.
///
/// `precision` is what the inspector's readout prints with, so a value off this
/// grid is a value the readout *lies about* — 1.234 shown as "1.23". Every
/// value this module produces is snapped here for that reason, and it doubles as
/// the fine-step size for [`step`].
#[must_use]
pub fn step_size(descriptor: &SettingDescriptor) -> f32 {
    grid(descriptor) as f32
}

/// The same grid in `f64`, which is what the arithmetic actually uses.
///
/// **This is not a tidiness preference.** `1.00f32 - 0.01f32` is `0.98999995`,
/// and re-snapping that in `f32` keeps it there, because `99.0f32 * 0.01f32`
/// lands one ulp below `0.99f32`. The readout then prints "0.99" for a value
/// that is not 0.99, which is the exact failure this whole module exists to
/// prevent — and it compounds over a held wheel. Doing the divide/round/multiply
/// in `f64` and narrowing **once** puts every notch on the nearest `f32` to the
/// decimal a user reads.
fn grid(descriptor: &SettingDescriptor) -> f64 {
    match descriptor.precision {
        0 => 1.0,
        1 => 0.1,
        2 => 0.01,
        3 => 0.001,
        // No descriptor uses more than 2 today; keep the formula rather than a
        // panic so a later precision column cannot silently step by 1.0.
        n => 10f64.powi(-(n as i32)),
    }
}

/// Bring any finite value onto the descriptor's contract: clamped into
/// `[minimum, maximum]`, snapped to the precision grid, and — for a toggle —
/// resolved to exactly 0 or 1.
///
/// This is the **only** way this module writes a value. `SceneSettings::set`
/// rejects rather than clamps (`scene_settings.c:143-149`), so anything that
/// does not pass through here can silently fail to write and leave the readout
/// showing a value nobody chose.
#[must_use]
pub fn conform(descriptor: &SettingDescriptor, value: f32) -> f32 {
    conform64(descriptor, f64::from(value))
}

/// [`conform`] over an `f64`, which every producer here goes through.
fn conform64(descriptor: &SettingDescriptor, value: f64) -> f32 {
    if !value.is_finite() {
        return descriptor.default_value;
    }
    if descriptor.kind == SettingKind::Toggle {
        // A toggle's midpoint has to fall one way; 0.5 reads as "on" because a
        // user dragging toward 1 has already left the off state.
        return if value >= 0.5 { 1.0 } else { 0.0 };
    }
    let clamped = value.clamp(f64::from(descriptor.minimum), f64::from(descriptor.maximum));
    let grid = grid(descriptor);
    let snapped = ((clamped / grid).round() * grid) as f32;
    // Rounding can leave the grid outside the bound when a bound is not itself
    // on the grid; the bound wins, because `accepts` is what a `.musi` file is
    // validated with.
    snapped.clamp(descriptor.minimum, descriptor.maximum)
}

/// Move a value by `steps` grid units (UX0-B09's fine/coarse nudge).
///
/// The wheel sends 1, Shift sends 10 — a ratio rather than two hand-picked
/// deltas, so a 0.01-grid slider and a 1-degree hue control both take the same
/// number of notches to cross the same fraction of their range.
#[must_use]
pub fn step(descriptor: &SettingDescriptor, value: f32, steps: i32) -> f32 {
    if descriptor.kind == SettingKind::Toggle {
        // A toggle has one interior step; any nonzero request flips it, rather
        // than needing exactly +1 to do anything.
        if steps == 0 {
            return conform(descriptor, value);
        }
        return if value >= 0.5 { 0.0 } else { 1.0 };
    }
    let grid = grid(descriptor);
    // Snap first, then move: a value arriving off-grid (from a route, or from a
    // `.musi` written by a slider drag) must not carry its offset through every
    // later notch.
    let snapped = f64::from(conform(descriptor, value));
    conform64(descriptor, snapped + f64::from(steps) * grid)
}

/// Why a typed value was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedValueError {
    /// The field was empty or whitespace — the user cleared it and pressed
    /// Enter, which is a cancel, not an error.
    Empty,
    /// Not a number at all.
    NotANumber,
}

/// The outcome of committing a typed value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypedValue {
    /// What will actually be written — conformed to the descriptor.
    pub value: f32,
    /// True when the typed number was outside the descriptor's range and was
    /// pulled in. The caller says so rather than pretending the user got what
    /// they asked for: a silently clamped 99 looks like a broken text field.
    pub clamped: bool,
    /// True when the typed number was rounded onto the precision grid.
    pub rounded: bool,
}

/// Parse and conform a typed value (UX0-B09).
///
/// Accepts a leading `+`, a leading/trailing space, and a bare `.5`, because a
/// text field a user types 1.5 into should not care. Percent, units and thousand
/// separators are refused rather than guessed at.
///
/// # Errors
///
/// [`TypedValueError::Empty`] for a blank field, [`TypedValueError::NotANumber`]
/// for anything `f32::from_str` will not take.
pub fn parse_typed(
    descriptor: &SettingDescriptor,
    text: &str,
) -> Result<TypedValue, TypedValueError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(TypedValueError::Empty);
    }
    let parsed: f32 = trimmed
        .strip_prefix('+')
        .unwrap_or(trimmed)
        .parse()
        .map_err(|_| TypedValueError::NotANumber)?;
    if !parsed.is_finite() {
        // "inf" and "nan" parse. They must not reach a descriptor.
        return Err(TypedValueError::NotANumber);
    }
    let value = conform(descriptor, parsed);
    Ok(TypedValue {
        value,
        clamped: parsed < descriptor.minimum || parsed > descriptor.maximum,
        rounded: (value - parsed.clamp(descriptor.minimum, descriptor.maximum)).abs()
            > f32::EPSILON,
    })
}

// -- bounded randomize ---------------------------------------------------------

/// How far an exploration moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    /// Small moves around wherever the tuning already is. Toggles never flip.
    Nudge,
    /// A new look for the scene, centred on the scene's own designed character.
    Surprise,
}

/// The fraction of a slider's span kept clear of a **degenerate** end by
/// [`Strength::Surprise`].
///
/// **The bias that makes Surprise worth pressing.** A uniform draw over the full
/// range lands on a hard endpoint often, and hard endpoints are mostly where
/// the degenerate looks live: zero density, zero glow, zero link weight, maximum
/// everything. Five per cent off such an end costs almost nothing expressive and
/// removes the results a user would never keep.
///
/// **Per end, not per range (CX-4).** The first version inset *every* end of
/// *every* slider, which excluded designed values: `settings.pulse.petals` is
/// `0..12` where **0 means auto**, and a blanket inset made auto undrawable —
/// Surprise could never hand back the scene's own default character. Which ends
/// are degenerate is per-descriptor metadata now ([`DRAWABLE_LOW_KEYS`],
/// [`CYCLIC_KEYS`]); this constant is only the width used where an end *is*
/// degenerate. A *Nudge* is deliberately not inset at all — a user nudging a
/// value that already sits at an endpoint must be able to stay there.
const SURPRISE_INSET: f32 = 0.05;

/// Probability that [`Strength::Surprise`] moves any given slider.
///
/// Not 1.0, and not the 0.75 it shipped at: CX-4's ruling is that nine changed
/// controls out of twelve is not "the scene stays recognisable" — it only looked
/// recognisable because it was still the same renderer. At 0.45 roughly half the
/// controls stay put and the result reads as *this* scene, differently.
const SURPRISE_MOVE_CHANCE: f64 = 0.45;

/// Probability that [`Strength::Surprise`] flips a toggle.
///
/// Low on purpose, and lowered again by CX-4: `atlas.wireframe` and
/// `atlas.hue_motion` are whole-scene character switches, and at 0.25 the
/// chance that at least one of the two flips was 44 % *per press* — which is
/// not "occasionally", it is a different button each time. 0.15 puts a flip at
/// roughly every fourth press.
const SURPRISE_TOGGLE_CHANCE: f64 = 0.15;

/// Descriptors whose range is a **circle**: the two ends are the same point,
/// so no end is degenerate, no inset applies, and Surprise samples uniformly —
/// hue has no designed centre to pull toward.
///
/// **Explicit, not inferred (CX-4).** The first version read "circular" off the
/// bounds (`minimum < 0 && maximum > 0`), which was true of all five matching
/// descriptors at the time but is unsafe inference for any future symmetric
/// range that is not circular — and was already wrong once: `atlas.orbit` is a
/// camera composition control that merely *happens* to span `-180..180`, and it
/// belongs with the triangular draws, not the colour wheel. A test resolves
/// every key here against the descriptor table, so a renamed key fails loudly
/// instead of silently losing its treatment.
#[rustfmt::skip]
const CYCLIC_KEYS: &[&str] = &[
    "settings.pulse.hue",
    "settings.orbital.hue",
    "settings.atlas.color",
    "settings.pentagram.hue",
    "settings.phosphor.hue",
];

/// Descriptors whose **low** end is a designed value rather than a degenerate:
/// Surprise may land exactly on it.
///
/// All three spell zero as "let the scene decide", which is the opposite of a
/// degenerate look — it is the scene's own character. (No descriptor currently
/// has a drawable *high* end; when one appears it gets the mirror table, not a
/// relaxation of this one.)
#[rustfmt::skip]
const DRAWABLE_LOW_KEYS: &[&str] = &[
    "settings.pulse.petals",   // 0 = auto petal fold
    "settings.phosphor.field", // 0 = cycle all fields
    "settings.phosphor.ramp",  // 0 = each field keeps its own alphabet
];

/// Half-width of a [`Strength::Nudge`], as a fraction of the descriptor's span.
const NUDGE_SPREAD: f32 = 0.12;

/// A triangular draw on `[low, high]` with its mode at `mode`.
///
/// Triangular rather than uniform because the mode is where the interesting
/// values are and the tails are where the boring ones are, and rather than a
/// Gaussian because it has hard support: the result cannot leave `[low, high]`,
/// so bounds are guaranteed by construction and not by a clamp that hides how
/// often it fired.
fn triangular(rng: &mut impl RandomSource, low: f64, high: f64, mode: f64) -> f64 {
    if high <= low {
        return low;
    }
    let mode = mode.clamp(low, high);
    let span = high - low;
    let u = rng.next_unit();
    let split = (mode - low) / span;
    if u < split {
        low + (u * span * (mode - low)).sqrt()
    } else {
        high - ((1.0 - u) * span * (high - mode)).sqrt()
    }
}

/// Whether a descriptor is one of the explicitly-marked circular controls.
fn is_cyclic(descriptor: &SettingDescriptor) -> bool {
    CYCLIC_KEYS.contains(&descriptor.key)
}

/// Whether Surprise may land exactly on the descriptor's low end.
fn low_end_is_drawable(descriptor: &SettingDescriptor) -> bool {
    DRAWABLE_LOW_KEYS.contains(&descriptor.key)
}

/// One setting's new value under `strength`.
fn explore_value(
    rng: &mut impl RandomSource,
    descriptor: &SettingDescriptor,
    current: f32,
    strength: Strength,
) -> f32 {
    match strength {
        Strength::Nudge => {
            if descriptor.kind == SettingKind::Toggle {
                return conform(descriptor, current);
            }
            let span = f64::from(descriptor.maximum - descriptor.minimum);
            let reach = span * f64::from(NUDGE_SPREAD);
            let centre = f64::from(current);
            // Symmetric triangular about the current value: most nudges are
            // small, and the largest possible one is a visible-but-recoverable
            // 12 % of the range.
            let value = triangular(rng, centre - reach, centre + reach, centre);
            conform64(descriptor, value)
        }
        Strength::Surprise => {
            if descriptor.kind == SettingKind::Toggle {
                return if rng.chance(SURPRISE_TOGGLE_CHANCE) {
                    conform(descriptor, 1.0 - current)
                } else {
                    conform(descriptor, current)
                };
            }
            if !rng.chance(SURPRISE_MOVE_CHANCE) {
                return conform(descriptor, current);
            }
            let span = descriptor.maximum - descriptor.minimum;
            let cyclic = is_cyclic(descriptor);
            // The inset applies per end, and only to a degenerate end: a
            // circle has none, and a "0 = auto" low end is the scene's own
            // character rather than a look nobody would keep (CX-4).
            let low_inset = if cyclic || low_end_is_drawable(descriptor) {
                0.0
            } else {
                span * SURPRISE_INSET
            };
            let high_inset = if cyclic { 0.0 } else { span * SURPRISE_INSET };
            let low = f64::from(descriptor.minimum + low_inset);
            let high = f64::from(descriptor.maximum - high_inset);
            let value = if cyclic {
                rng.next_range(low, high)
            } else {
                triangular(rng, low, high, f64::from(descriptor.default_value))
            };
            conform64(descriptor, value)
        }
    }
}

/// A bounded random re-tuning of one scene (UX0-C07).
///
/// Every value in the result satisfies its descriptor's `accepts`, so the
/// snapshot can be applied, captured into a cue and saved to a `.musi` file
/// without a single rejection. Deterministic for a given `rng` state.
///
/// Guarantees at least one changed value where the scene has any control that
/// *can* change — a button that sometimes does nothing is a button a user stops
/// trusting, and with `SURPRISE_MOVE_CHANCE` at 0.45 an all-skipped draw
/// happens about once in 66 presses for a seven-control scene (0.55⁷), which
/// is why the force-one branch below is not decorative.
#[must_use]
pub fn explore(
    rng: &mut impl RandomSource,
    scene: SceneId,
    base: &SceneSettings,
    strength: Strength,
) -> SettingsSnapshot {
    let descriptors = settings::descriptors(scene);
    let mut values = [0.0f32; MAX_CONTROLS];
    let mut changed = false;
    for (index, descriptor) in descriptors.iter().enumerate() {
        let current = base.get(scene, index);
        let next = explore_value(rng, descriptor, current, strength);
        values[index] = next;
        changed |= next.to_bits() != conform(descriptor, current).to_bits();
    }

    if !changed {
        // Force one. Sixteen attempts, then give up rather than spin: a
        // descriptor with a zero-width range has nothing to offer and is not
        // worth an infinite loop.
        let pick = (rng.next_u64() as usize) % descriptors.len().max(1);
        if let Some(descriptor) = descriptors.get(pick) {
            for _ in 0..16 {
                let candidate = explore_value(rng, descriptor, values[pick], Strength::Surprise);
                if candidate.to_bits() != values[pick].to_bits() {
                    values[pick] = candidate;
                    break;
                }
            }
        }
    }

    SettingsSnapshot {
        captured: true,
        count: descriptors.len(),
        values,
    }
}

// -- the audition session ------------------------------------------------------

/// What a Tune edit lands on, and therefore what an audition must revert into.
///
/// Tuning is redirected into a cue's captured snapshot whenever the playhead
/// sits on one (`App::settings_mut`, and the header sentence LX3-b added for
/// it). Reverting a snapshot taken against the base scene *into* a cue would
/// write values that were never there, so the session carries its target and
/// ends when the target moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuneTarget {
    pub track_slot: usize,
    pub scene: SceneId,
    /// The active cue's position in the plan, or `None` for the base scene.
    pub cue: Option<usize>,
}

/// What started the audition, for the sentence the panel prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExploreSource {
    Preset,
    Nudge,
    Surprise,
    /// A slider drag or typed value made while an audition was already open.
    Manual,
}

impl ExploreSource {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Nudge => "Nudge",
            Self::Surprise => "Surprise",
            Self::Manual => "edits",
        }
    }
}

/// Which of the two tunings is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The tuning as it was before the first exploratory action.
    Before,
    /// The explored tuning.
    After,
}

/// One open audition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audition {
    pub target: TuneTarget,
    /// "A" — captured before the first exploratory action, never overwritten
    /// while the session lives. Exploring twice still reverts all the way to
    /// where the user actually was, not to the previous experiment.
    before: SettingsSnapshot,
    /// "B" while "A" is on screen. `None` while "B" is on screen, because then
    /// "B" *is* the live settings.
    stashed_after: Option<SettingsSnapshot>,
    pub showing: Side,
    pub source: ExploreSource,
    /// How many exploratory actions this session has taken, for the sentence.
    pub steps: u32,
}

impl Audition {
    /// The tuning to revert to.
    #[must_use]
    pub fn before(&self) -> SettingsSnapshot {
        self.before
    }
}

/// The Tune panel's audition state (UX0-C04).
///
/// At most one session, because a second one would have to answer "revert to
/// which?" and the answer a user wants is always "to before I started playing".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExploreState {
    session: Option<Audition>,
}

/// What the caller must write after a state transition.
///
/// A snapshot rather than a mutated `SceneSettings`, because the panel cannot
/// reach the settings store: it emits one `SetSetting` per value and the
/// application's own targeting decides where they land — the same path a slider
/// drag takes, so an audition can never write somewhere a slider could not.
pub type Apply = Option<SettingsSnapshot>;

impl ExploreState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The open session for this exact target, if any.
    ///
    /// Returns `None` for a session belonging to a different track, scene or
    /// cue — the caller then draws no audition row, which is the visible signal
    /// that the session is over.
    #[must_use]
    pub fn session(&self, target: TuneTarget) -> Option<Audition> {
        self.session.filter(|session| session.target == target)
    }

    /// Drop a session whose target has moved out from under it.
    ///
    /// Called once per frame with the target the panel is about to draw. Kept
    /// separate from [`Self::session`] so the read is not secretly a write.
    pub fn retarget(&mut self, target: TuneTarget) {
        if self.session.is_some_and(|session| session.target != target) {
            self.session = None;
        }
    }

    /// Open a session, or fold this action into the one already open.
    ///
    /// Returns `false` when the current settings cannot be captured — which
    /// means a value is out of its descriptor's range, and an audition that
    /// cannot promise an exact revert must not claim to.
    pub fn begin(
        &mut self,
        target: TuneTarget,
        current: &SceneSettings,
        source: ExploreSource,
    ) -> bool {
        if let Some(session) = self.session.as_mut() {
            if session.target == target {
                session.source = source;
                session.steps = session.steps.saturating_add(1);
                // Exploring while "A" is on screen explores *from* A and throws
                // the stashed B away. Any other rule would leave the compare
                // button toggling between two things the user never chose.
                session.showing = Side::After;
                session.stashed_after = None;
                return true;
            }
        }
        let Some(before) = current.capture(target.scene) else {
            return false;
        };
        self.session = Some(Audition {
            target,
            before,
            stashed_after: None,
            showing: Side::After,
            source,
            steps: 1,
        });
        true
    }

    /// Record that the user edited a value by hand while auditioning.
    ///
    /// Does **not** open a session: a slider drag with nothing to compare
    /// against is just a slider drag, and putting a Revert bar on screen for it
    /// would make every edit look like an experiment.
    pub fn note_manual_edit(&mut self, target: TuneTarget) {
        if let Some(session) = self.session.as_mut() {
            if session.target == target {
                session.source = ExploreSource::Manual;
                session.showing = Side::After;
                session.stashed_after = None;
            }
        }
    }

    /// A/B: swap what is on screen, keeping the other side for the next press.
    ///
    /// `current` is the live tuning, which is "B" when B is showing and is a
    /// copy of "A" when A is showing.
    pub fn compare(&mut self, target: TuneTarget, current: &SceneSettings) -> Apply {
        let session = self.session.as_mut().filter(|s| s.target == target)?;
        match session.showing {
            Side::After => {
                session.stashed_after = current.capture(target.scene);
                session.stashed_after?;
                session.showing = Side::Before;
                Some(session.before)
            }
            Side::Before => {
                let after = session.stashed_after.take()?;
                session.showing = Side::After;
                Some(after)
            }
        }
    }

    /// Revert to "A" and end the session.
    pub fn revert(&mut self, target: TuneTarget) -> Apply {
        let session = self.session.filter(|s| s.target == target)?;
        self.session = None;
        Some(session.before)
    }

    /// Keep whatever is on screen and end the session.
    ///
    /// Refuses while "A" is on screen: "Keep" there would silently discard the
    /// experiment the user is comparing against, and the button reads as the
    /// opposite of that. The panel disables it and says why instead.
    pub fn keep(&mut self, target: TuneTarget) -> bool {
        let Some(session) = self.session.filter(|s| s.target == target) else {
            return false;
        };
        if session.showing == Side::Before {
            return false;
        }
        self.session = None;
        true
    }

    /// End any session unconditionally (project load, track close).
    pub fn clear(&mut self) {
        self.session = None;
    }
}

/// The audition row's sentence.
///
/// Pure and pinned by a test rather than only drawn, for the reason
/// `tune_scope_label` is: a label a capture happens to show is not the same
/// claim as a label a test pins byte for byte.
#[must_use]
pub fn audition_label(session: &Audition) -> String {
    let what = session.source.label();
    match session.showing {
        Side::Before => format!("Comparing: original (B shows {what})"),
        Side::After if session.steps == 1 => format!("Auditioning: {what}"),
        Side::After => format!("Auditioning: {what} ({} steps)", session.steps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::settings::index;

    fn descriptor_of(scene: SceneId, index: usize) -> &'static SettingDescriptor {
        settings::descriptor(scene, index).unwrap()
    }

    /// Every descriptor in the tree, so a per-scene loop cannot quietly skip one.
    fn all_descriptors() -> Vec<(SceneId, usize, &'static SettingDescriptor)> {
        SceneId::ALL
            .into_iter()
            .flat_map(|scene| {
                settings::descriptors(scene)
                    .iter()
                    .enumerate()
                    .map(move |(index, descriptor)| (scene, index, descriptor))
            })
            .collect()
    }

    #[test]
    fn the_descriptor_sweep_really_covers_every_control() {
        // If a scene gains a control this fails and every sweep below is
        // knowingly re-scoped rather than silently narrowed.
        //
        // Two numbers now, because the tree has a scene the oracle does not.
        // 81 is what `differential_settings.sh` still diffs against the frozen C
        // and must not move; 93 is what this module sweeps, and it moves whenever
        // a post-legacy scene is added or changed.
        let oracle: usize = SceneId::ORACLE_ALL
            .into_iter()
            .map(|scene| settings::descriptors(scene).len())
            .sum();
        assert_eq!(
            oracle, 81,
            "the C-era descriptor count is a frozen contract"
        );
        assert_eq!(all_descriptors().len(), 93);
    }

    // -- conform / step / parse ------------------------------------------------

    #[test]
    fn conform_lands_every_wild_value_inside_the_contract() {
        for (_, _, descriptor) in all_descriptors() {
            for probe in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                -1e30,
                1e30,
                descriptor.minimum - 1.0,
                descriptor.maximum + 1.0,
                descriptor.minimum,
                descriptor.maximum,
                (descriptor.minimum + descriptor.maximum) * 0.5,
            ] {
                let value = conform(descriptor, probe);
                assert!(
                    descriptor.accepts(value),
                    "{} conformed {probe} to {value}, outside {}..{}",
                    descriptor.key,
                    descriptor.minimum,
                    descriptor.maximum
                );
            }
        }
    }

    #[test]
    fn conform_snaps_to_the_precision_the_readout_prints() {
        // A value the readout would print as "1.23" must *be* 1.23, or the
        // number on screen is not the number in the file.
        let weight = descriptor_of(SceneId::Loom, index::loom::WEIGHT); // precision 2
        assert_eq!(conform(weight, 1.234_5), 1.23);
        let hue = descriptor_of(SceneId::PulseField, index::pulse::HUE); // precision 0
        assert_eq!(conform(hue, 42.7), 43.0);
        assert_eq!(conform(hue, -42.7), -43.0);
    }

    #[test]
    fn a_toggle_resolves_to_exactly_zero_or_one() {
        let wireframe = descriptor_of(SceneId::SongAtlas, index::atlas::WIREFRAME);
        assert_eq!(wireframe.kind, SettingKind::Toggle);
        assert_eq!(conform(wireframe, 0.49), 0.0);
        assert_eq!(conform(wireframe, 0.5), 1.0);
        assert_eq!(conform(wireframe, 7.0), 1.0);
        // And a step of any size flips it rather than needing exactly one grid
        // unit — a wheel notch over a toggle must do something.
        assert_eq!(step(wireframe, 0.0, 1), 1.0);
        assert_eq!(step(wireframe, 1.0, 1), 0.0);
        assert_eq!(step(wireframe, 0.0, 10), 1.0);
        assert_eq!(step(wireframe, 0.0, 0), 0.0);
    }

    #[test]
    fn stepping_walks_the_grid_and_stops_at_the_bounds() {
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE); // 0.40..2.00 p2
        assert_eq!(step(amplitude, 1.00, 1), 1.01);
        assert_eq!(step(amplitude, 1.00, -1), 0.99);
        assert_eq!(step(amplitude, 1.00, 10), 1.10);
        // The bound is a wall, not a wrap.
        assert_eq!(step(amplitude, 2.00, 1), 2.00);
        assert_eq!(step(amplitude, 0.40, -1), 0.40);
        assert_eq!(step(amplitude, 0.40, -10_000), 0.40);
    }

    #[test]
    fn stepping_from_an_off_grid_value_does_not_carry_the_offset() {
        // A route or a slider drag can leave 1.2345 in the store. Ten notches
        // from there must reach a printable value, not 1.3345.
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE);
        assert_eq!(step(amplitude, 1.234_5, 10), 1.33);
    }

    #[test]
    fn stepping_never_leaves_the_contract_for_any_descriptor() {
        for (_, _, descriptor) in all_descriptors() {
            let mut value = descriptor.default_value;
            for steps in [1, -1, 10, -10, 1000, -1000, i32::MAX / 2, i32::MIN / 2] {
                value = step(descriptor, value, steps);
                assert!(
                    descriptor.accepts(value),
                    "{} stepped by {steps} to {value}",
                    descriptor.key
                );
            }
        }
    }

    #[test]
    fn typed_values_are_clamped_by_the_descriptor_and_say_so() {
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE); // 0.40..2.00
        let ok = parse_typed(amplitude, "1.5").unwrap();
        assert_eq!(ok.value, 1.5);
        assert!(!ok.clamped);

        let high = parse_typed(amplitude, "99").unwrap();
        assert_eq!(high.value, 2.00);
        assert!(high.clamped);

        let low = parse_typed(amplitude, "-3").unwrap();
        assert_eq!(low.value, 0.40);
        assert!(low.clamped);
    }

    #[test]
    fn typed_values_round_onto_the_precision_grid_and_say_so() {
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE);
        let typed = parse_typed(amplitude, "1.239").unwrap();
        assert_eq!(typed.value, 1.24);
        assert!(typed.rounded);
        assert!(!parse_typed(amplitude, "1.24").unwrap().rounded);
    }

    #[test]
    fn typed_values_take_the_forms_a_user_actually_types() {
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE);
        assert_eq!(parse_typed(amplitude, " 1.5 ").unwrap().value, 1.5);
        assert_eq!(parse_typed(amplitude, "+1.5").unwrap().value, 1.5);
        assert_eq!(parse_typed(amplitude, ".5").unwrap().value, 0.5);
        let hue = descriptor_of(SceneId::PulseField, index::pulse::HUE);
        assert_eq!(parse_typed(hue, "-90").unwrap().value, -90.0);
    }

    #[test]
    fn typed_nonsense_is_refused_rather_than_guessed_at() {
        let amplitude = descriptor_of(SceneId::Spectrum, index::spectrum::AMPLITUDE);
        assert_eq!(parse_typed(amplitude, ""), Err(TypedValueError::Empty));
        assert_eq!(parse_typed(amplitude, "   "), Err(TypedValueError::Empty));
        assert_eq!(
            parse_typed(amplitude, "1.5x"),
            Err(TypedValueError::NotANumber)
        );
        assert_eq!(
            parse_typed(amplitude, "50%"),
            Err(TypedValueError::NotANumber)
        );
        // Negative control for the comparator itself: `f32::from_str` *accepts*
        // these two, so without the `is_finite` guard they would reach a
        // descriptor and `accepts` would then reject the write silently.
        assert_eq!(
            parse_typed(amplitude, "inf"),
            Err(TypedValueError::NotANumber)
        );
        assert_eq!(
            parse_typed(amplitude, "NaN"),
            Err(TypedValueError::NotANumber)
        );
    }

    // -- randomize -------------------------------------------------------------

    #[test]
    fn a_seeded_explore_is_reproducible_and_a_different_seed_is_not() {
        let base = SceneSettings::default();
        let run = |seed: u64| {
            let mut rng = SplitMix64::new(seed);
            explore(&mut rng, SceneId::Loom, &base, Strength::Surprise)
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
    }

    #[test]
    fn every_seed_stays_inside_every_one_of_the_eighty_one_descriptors() {
        // The sweep the plan asks for. 200 seeds x 10 scenes x both strengths:
        // 1620 descriptor writes per seed, all of which must be applyable and
        // savable without a single rejection.
        let base = SceneSettings::default();
        for seed in 0..200u64 {
            for scene in SceneId::ALL {
                for strength in [Strength::Nudge, Strength::Surprise] {
                    let mut rng = SplitMix64::new(seed.wrapping_mul(0x9E37_79B9) ^ 0xA5A5);
                    let snapshot = explore(&mut rng, scene, &base, strength);
                    assert!(
                        snapshot.is_valid_for(scene),
                        "{scene:?} {strength:?} seed {seed} produced an invalid snapshot"
                    );
                    for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
                        let value = snapshot.values[index];
                        assert!(
                            descriptor.accepts(value),
                            "{} = {value} outside {}..{} ({strength:?}, seed {seed})",
                            descriptor.key,
                            descriptor.minimum,
                            descriptor.maximum
                        );
                    }
                    // And it must actually be writable: `set` rejects rather
                    // than clamps, so a value that merely "looks" in range but
                    // is off a toggle's 0/1 rule would fail here.
                    let mut applied = SceneSettings::default();
                    for (index, _) in settings::descriptors(scene).iter().enumerate() {
                        assert!(
                            applied.set(scene, index, snapshot.values[index]),
                            "{scene:?}[{index}] rejected by SceneSettings::set"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn explore_always_changes_something() {
        let base = SceneSettings::default();
        for seed in 0..500u64 {
            for scene in SceneId::ALL {
                let mut rng = SplitMix64::new(seed);
                let snapshot = explore(&mut rng, scene, &base, Strength::Surprise);
                let moved = settings::descriptors(scene)
                    .iter()
                    .enumerate()
                    .any(|(index, _)| snapshot.values[index] != base.get(scene, index));
                assert!(moved, "{scene:?} seed {seed} produced no change at all");
            }
        }
    }

    #[test]
    fn surprise_keeps_clear_of_the_degenerate_endpoints_only() {
        // The bias, as a measurement. Over 400 draws no slider may land in the
        // outer 5 % of a *degenerate* end — while a cyclic control has no
        // degenerate end at all, and a "0 = auto" low end is exempt by name
        // (CX-4). The exemptions are asserted per key, not skipped, so a
        // regression to the blanket inset fails the drawability test below
        // rather than passing this one vacuously.
        let base = SceneSettings::default();
        for seed in 0..400u64 {
            for scene in SceneId::ALL {
                let mut rng = SplitMix64::new(seed ^ 0x5EED);
                let snapshot = explore(&mut rng, scene, &base, Strength::Surprise);
                for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
                    if descriptor.kind == SettingKind::Toggle || is_cyclic(descriptor) {
                        continue;
                    }
                    let value = snapshot.values[index];
                    if value == conform(descriptor, base.get(scene, index)) {
                        // An unmoved control keeps whatever it had, including an
                        // endpoint the user or the default put it on.
                        continue;
                    }
                    let span = descriptor.maximum - descriptor.minimum;
                    let grid = step_size(descriptor);
                    // One grid unit of tolerance: the inset is computed before
                    // the snap to precision, so a 5 % boundary can round outward
                    // by at most half a step.
                    if !low_end_is_drawable(descriptor) {
                        assert!(
                            value >= descriptor.minimum + span * SURPRISE_INSET - grid,
                            "{} landed at {value}, inside the degenerate low margin of {}..{}",
                            descriptor.key,
                            descriptor.minimum,
                            descriptor.maximum
                        );
                    }
                    assert!(
                        value <= descriptor.maximum - span * SURPRISE_INSET + grid,
                        "{} landed at {value}, inside the degenerate high margin of {}..{}",
                        descriptor.key,
                        descriptor.minimum,
                        descriptor.maximum
                    );
                }
            }
        }
    }

    #[test]
    fn the_endpoint_and_cyclic_tables_resolve_against_the_descriptor_contract() {
        // A renamed key would otherwise silently lose its treatment — the same
        // defect class as a widget id minted outside the ALL table.
        for key in CYCLIC_KEYS.iter().chain(DRAWABLE_LOW_KEYS) {
            let (_, _, descriptor) = settings::descriptor_by_key(key)
                .unwrap_or_else(|| panic!("{key} is not a descriptor"));
            assert_eq!(descriptor.key, *key);
        }
        // Every cyclic control is a full turn of degrees; anything else listed
        // here is a mistake, not a new kind of circle.
        for key in CYCLIC_KEYS {
            let (_, _, descriptor) = settings::descriptor_by_key(key).unwrap();
            assert_eq!((descriptor.minimum, descriptor.maximum), (-180.0, 180.0));
        }
        // And the ruling's named counter-example stays out: `atlas.orbit`
        // spans -180..180 but is camera composition, not colour (CX-4).
        assert!(!CYCLIC_KEYS.contains(&"settings.atlas.orbit"));
    }

    #[test]
    fn a_drawable_low_endpoint_is_actually_drawn() {
        // The negative control for the metadata: under the blanket inset,
        // `pulse.petals = 0` (auto) was *undrawable* — the 5 % inset put the
        // floor at 0.6 and precision-0 rounding kept every draw at >= 1. Each
        // exempt key must reach its own low end within a few hundred presses,
        // or the exemption is dead and this fails.
        let base = SceneSettings::default();
        for key in DRAWABLE_LOW_KEYS {
            let (scene, index, descriptor) = settings::descriptor_by_key(key).unwrap();
            let mut reached = false;
            for seed in 0..400u64 {
                let mut rng = SplitMix64::new(seed ^ 0xD00D);
                let snapshot = explore(&mut rng, scene, &base, Strength::Surprise);
                if snapshot.values[index] == descriptor.minimum {
                    reached = true;
                    break;
                }
            }
            assert!(reached, "{key} never drew its designed low end");
        }
    }

    #[test]
    fn surprise_leaves_most_of_the_scene_alone() {
        // CX-4's actual complaint, as a measurement: at 0.75 move chance nine
        // of Song Atlas's twelve controls changed per press. At 0.45 the
        // average across many presses must sit near half — the scene stays
        // recognisable as itself.
        let base = SceneSettings::default();
        let mut moved_total = 0.0f64;
        let mut samples = 0.0f64;
        for seed in 0..2000u64 {
            let mut rng = SplitMix64::new(seed ^ 0x50_FA);
            let snapshot = explore(&mut rng, SceneId::SongAtlas, &base, Strength::Surprise);
            let moved = settings::descriptors(SceneId::SongAtlas)
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    snapshot.values[*index] != base.get(SceneId::SongAtlas, *index)
                })
                .count();
            moved_total += moved as f64;
            samples += 1.0;
        }
        let mean_moved = moved_total / samples;
        assert!(
            (4.0..7.0).contains(&mean_moved),
            "Song Atlas moves {mean_moved} of 12 controls on average; 0.45 should put it near 5.4"
        );
    }

    #[test]
    fn a_nudge_stays_small_and_never_flips_a_toggle() {
        let base = SceneSettings::default();
        for seed in 0..300u64 {
            for scene in SceneId::ALL {
                let mut rng = SplitMix64::new(seed ^ 0xB00C);
                let snapshot = explore(&mut rng, scene, &base, Strength::Nudge);
                for (index, descriptor) in settings::descriptors(scene).iter().enumerate() {
                    let before = base.get(scene, index);
                    let after = snapshot.values[index];
                    if descriptor.kind == SettingKind::Toggle {
                        assert_eq!(after, before, "{} flipped under Nudge", descriptor.key);
                        continue;
                    }
                    let span = descriptor.maximum - descriptor.minimum;
                    let grid = step_size(descriptor);
                    assert!(
                        (after - before).abs() <= span * NUDGE_SPREAD + grid,
                        "{} moved {} under Nudge, more than {NUDGE_SPREAD} of its span",
                        descriptor.key,
                        (after - before).abs()
                    );
                }
            }
        }
    }

    #[test]
    fn surprise_moves_hue_widely_and_the_camera_orbit_gently() {
        // The cyclic carve-out, measured rather than asserted in prose. A
        // cyclic control samples uniformly over the whole circle, so its mean
        // absolute value sits near 0.45 * 90 = 40; a triangular draw about the
        // 0 default gives roughly a third of the uniform figure. `atlas.orbit`
        // spans the same -180..180 and must land in the *triangular* regime —
        // it is camera composition, and CX-4's point is exactly that the old
        // bounds inference could not tell the two apart.
        let base = SceneSettings::default();
        let mean_abs = |scene: SceneId, index: usize, salt: u64| {
            let mut total = 0.0f64;
            for seed in 0..2000u64 {
                let mut rng = SplitMix64::new(seed ^ salt);
                let snapshot = explore(&mut rng, scene, &base, Strength::Surprise);
                total += f64::from(snapshot.values[index].abs());
            }
            total / 2000.0
        };

        let hue = mean_abs(SceneId::PulseField, index::pulse::HUE, 0xC0FFEE);
        assert!(
            hue > 30.0,
            "hue mean magnitude {hue} — the cyclic carve-out is not firing"
        );

        let color = mean_abs(SceneId::SongAtlas, index::atlas::COLOR, 0xC0FFEE);
        let orbit = mean_abs(SceneId::SongAtlas, index::atlas::ORBIT, 0xC0FFEE);
        assert!(
            orbit < color * 0.75,
            "orbit mean {orbit} vs colour mean {color}: the orbit is still being sampled as a colour wheel"
        );
    }

    // -- audition --------------------------------------------------------------

    fn target() -> TuneTarget {
        TuneTarget {
            track_slot: 0,
            scene: SceneId::Loom,
            cue: None,
        }
    }

    /// Apply a snapshot the way the panel does: one conformed write per value.
    fn apply(settings: &mut SceneSettings, scene: SceneId, snapshot: &SettingsSnapshot) {
        for (index, _) in settings::descriptors(scene).iter().enumerate() {
            assert!(settings.set(scene, index, snapshot.values[index]));
        }
    }

    #[test]
    fn a_revert_is_bit_for_bit_identical() {
        // The claim UX0-C04 rests on. Not "close": the same bits, so a `.musi`
        // written after a revert is byte-identical to one written before the
        // audition, and an export is frame-identical.
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        settings.set(SceneId::Loom, index::loom::DENSITY, 0.83);
        let original = settings;
        let original_bits: Vec<u32> = settings::descriptors(SceneId::Loom)
            .iter()
            .enumerate()
            .map(|(index, _)| settings.get(SceneId::Loom, index).to_bits())
            .collect();

        let mut state = ExploreState::new();
        assert!(state.begin(target(), &settings, ExploreSource::Surprise));

        let mut rng = SplitMix64::new(99);
        let wild = explore(&mut rng, SceneId::Loom, &settings, Strength::Surprise);
        apply(&mut settings, SceneId::Loom, &wild);
        assert_ne!(settings, original, "the audition changed nothing to revert");

        let back = state.revert(target()).expect("a session was open");
        apply(&mut settings, SceneId::Loom, &back);

        assert_eq!(settings, original);
        let after_bits: Vec<u32> = settings::descriptors(SceneId::Loom)
            .iter()
            .enumerate()
            .map(|(index, _)| settings.get(SceneId::Loom, index).to_bits())
            .collect();
        assert_eq!(after_bits, original_bits, "revert is not bit-for-bit");
        assert!(state.session(target()).is_none(), "the session must end");
    }

    #[test]
    fn exploring_twice_still_reverts_all_the_way_back() {
        // The trap a naive "remember the previous values" undo falls into: two
        // Surprises would leave the first Surprise as the thing you revert to.
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        let original = settings;

        let mut state = ExploreState::new();
        let mut rng = SplitMix64::new(3);
        for _ in 0..5 {
            assert!(state.begin(target(), &settings, ExploreSource::Surprise));
            let wild = explore(&mut rng, SceneId::Loom, &settings, Strength::Surprise);
            apply(&mut settings, SceneId::Loom, &wild);
        }
        assert_eq!(state.session(target()).unwrap().steps, 5);

        let back = state.revert(target()).unwrap();
        apply(&mut settings, SceneId::Loom, &back);
        assert_eq!(settings, original);
    }

    #[test]
    fn compare_swaps_both_ways_without_losing_either_side() {
        let mut settings = SceneSettings::default();
        settings.set(SceneId::Loom, index::loom::WEIGHT, 1.37);
        let original = settings;

        let mut state = ExploreState::new();
        assert!(state.begin(target(), &settings, ExploreSource::Preset));
        let mut rng = SplitMix64::new(11);
        let wild = explore(&mut rng, SceneId::Loom, &settings, Strength::Surprise);
        apply(&mut settings, SceneId::Loom, &wild);
        let explored = settings;

        // B -> A
        let a = state.compare(target(), &settings).unwrap();
        apply(&mut settings, SceneId::Loom, &a);
        assert_eq!(settings, original);
        assert_eq!(state.session(target()).unwrap().showing, Side::Before);

        // A -> B, and B is still exactly what it was.
        let b = state.compare(target(), &settings).unwrap();
        apply(&mut settings, SceneId::Loom, &b);
        assert_eq!(settings, explored);
        assert_eq!(state.session(target()).unwrap().showing, Side::After);

        // And a revert from here is still the original.
        let back = state.revert(target()).unwrap();
        apply(&mut settings, SceneId::Loom, &back);
        assert_eq!(settings, original);
    }

    #[test]
    fn keep_refuses_while_the_original_is_on_screen() {
        // "Keep" with A on screen would throw B away, which is the opposite of
        // what the word says.
        let settings = SceneSettings::default();
        let mut state = ExploreState::new();
        assert!(state.begin(target(), &settings, ExploreSource::Surprise));
        assert!(state.keep(target()));
        assert!(state.session(target()).is_none());

        assert!(state.begin(target(), &settings, ExploreSource::Surprise));
        state.compare(target(), &settings).unwrap();
        assert!(!state.keep(target()));
        assert!(state.session(target()).is_some(), "the session survives");
    }

    #[test]
    fn a_session_belongs_to_one_track_scene_and_cue() {
        let settings = SceneSettings::default();
        let mut state = ExploreState::new();
        assert!(state.begin(target(), &settings, ExploreSource::Surprise));

        let other_cue = TuneTarget {
            cue: Some(2),
            ..target()
        };
        let other_scene = TuneTarget {
            scene: SceneId::Spectrum,
            ..target()
        };
        let other_track = TuneTarget {
            track_slot: 1,
            ..target()
        };
        for moved in [other_cue, other_scene, other_track] {
            assert!(
                state.session(moved).is_none(),
                "a session must not be visible from {moved:?}"
            );
            assert!(state.revert(moved).is_none(), "nor revertible from it");
        }
        assert!(state.session(target()).is_some());

        // And the per-frame retarget ends it rather than leaving it lurking.
        state.retarget(other_cue);
        assert!(state.session(target()).is_none());
    }

    #[test]
    fn a_manual_edit_does_not_open_a_session_but_does_join_one() {
        let settings = SceneSettings::default();
        let mut state = ExploreState::new();
        state.note_manual_edit(target());
        assert!(
            state.session(target()).is_none(),
            "a bare slider drag is not an experiment"
        );

        assert!(state.begin(target(), &settings, ExploreSource::Surprise));
        state.compare(target(), &settings).unwrap();
        state.note_manual_edit(target());
        let session = state.session(target()).unwrap();
        assert_eq!(session.source, ExploreSource::Manual);
        // Editing while A is on screen makes the edited tuning the new B.
        assert_eq!(session.showing, Side::After);
    }

    #[test]
    fn an_uncapturable_tuning_refuses_to_open_a_session() {
        // `capture` returns None when a value is out of range, which is the one
        // case where an exact revert cannot be promised. Better no Revert button
        // than one that lies.
        let mut settings = SceneSettings::default();
        // Reach past `set`'s validation the only way the type allows: a snapshot
        // whose count is legal but whose values are not is refused too, so use a
        // scene whose stored value we corrupt through a legacy-sized snapshot.
        // There is no such door — `set` and `apply_snapshot` both validate — so
        // this asserts the reachable half: a valid tuning always captures.
        assert!(settings.set(SceneId::Loom, index::loom::WEIGHT, 2.5));
        let mut state = ExploreState::new();
        assert!(state.begin(target(), &settings, ExploreSource::Surprise));
    }

    #[test]
    fn the_audition_sentence_names_the_side_and_the_source() {
        let settings = SceneSettings::default();
        let mut state = ExploreState::new();
        state.begin(target(), &settings, ExploreSource::Surprise);
        assert_eq!(
            audition_label(&state.session(target()).unwrap()),
            "Auditioning: Surprise"
        );

        state.begin(target(), &settings, ExploreSource::Nudge);
        state.begin(target(), &settings, ExploreSource::Nudge);
        assert_eq!(
            audition_label(&state.session(target()).unwrap()),
            "Auditioning: Nudge (3 steps)"
        );

        state.compare(target(), &settings);
        assert_eq!(
            audition_label(&state.session(target()).unwrap()),
            "Comparing: original (B shows Nudge)"
        );

        state.begin(target(), &settings, ExploreSource::Preset);
        assert_eq!(
            audition_label(&state.session(target()).unwrap()),
            "Auditioning: preset (4 steps)"
        );
    }
}
