//! Glyph Lattice: a wireframe solid rasterised onto a hexagonal lattice of
//! tiny glyphs, lit by the music.
//!
//! The frame is a fixed honeycomb of cells, each able to show one small
//! glyph. A solid turning in front of it is drawn as thick strokes; a cell
//! inside a stroke lights, and how bright it is chooses its glyph — a faint
//! dot at the stroke's edge, rings and forks in its body, filled hexagons at
//! its core, sparks where light collects. Strokes that cross pool their light
//! into a warm bloom. Behind the subject, ghosts: the pose it had a moment
//! ago, the solid it just was, and two giants turning slowly in the dark.
//!
//! What the music does, and why:
//!
//! - **Kicks** fire pulses of light from a vertex along the solid's edges,
//!   hop by hop, so a hit is something you watch travel rather than a flash.
//!   They also lurch the spin forward a few degrees.
//! - **Track-relative loudness** sets the spin, the stroke weight and how
//!   much ghost there is: a quiet verse is a thin, slow solid; a chorus a
//!   heavy one with echoes.
//! - **The spectrum** lights the edges by height: the solid's lower edges
//!   carry the bass, its upper edges the treble. The knot carries it along its
//!   length.
//! - **Bass** swells the bloom; **treble** makes the glyphs flicker.
//! - **Onsets** send a ripple out from the centre; every glyph it passes
//!   changes shape, so the texture of the lattice turns over in waves.
//! - **The lyric line**, with `words` on, is set into the lattice in stroke
//!   capitals, stroke by stroke as it starts, and kicks sweep through it.
//!
//! The solid is a setting (`solid`), so a treatment picks one per section and
//! a change morphs: the new solid assembles outward from a vertex while the
//! old one exhales into the ghost layer. At 0 the scene cycles on its own,
//! moving on at a drop or after a long hold.
//!
//! Everything that moves is integrated here from the frame's track-time
//! delta; the lattice itself (`field`) is a pure function of this state, the
//! frame and the output's aspect, so preview, still and export agree and the
//! picture is a unit test.

pub mod field;
pub mod shapes;
pub mod strokes;

use super::aurora_veil::{follow, Features, KickDetector};
use crate::scene::settings::index::glyphs as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use shapes::Form;
use std::any::Any;
use std::f64::consts::TAU;

/// Pulses alive at once. A ninth replaces the oldest, which has run out by
/// then at any tempo a kick detector reports.
pub const MAX_PULSES: usize = 8;
/// Onset ripples alive at once.
pub const RIPPLES: usize = 8;
/// Band groups the spectrum is folded into for the edges.
pub const BAND_GROUPS: usize = 16;
/// How long the old solid takes to exhale into the ghost layer.
pub const FADE_SECONDS: f64 = 1.8;
/// How long a new solid takes to assemble outward from its first vertex.
pub const ASSEMBLE_SECONDS: f64 = 1.1;
/// A ripple's radius grows this many short sides per second.
pub const RIPPLE_SPEED: f32 = 0.95;
/// Seconds a ripple lives.
pub const RIPPLE_SECONDS: f64 = 1.6;
/// Angle history: one slot per 1/30 s of track time, a second long.
const HISTORY: usize = 32;
const HISTORY_SLOT: f64 = 1.0 / 30.0;
/// Without a kick for this long, the scene fires a pulse of its own, so a
/// quiet passage and `response` 0 still have light moving on the edges.
const AMBIENT_GAP: f32 = 2.6;
/// The automatic cycle moves on after this long on one solid, at a kick.
const AUTO_HOLD: f64 = 22.0;
/// Seconds of quiet before a loud kick counts as a drop.
const DROP_QUIET: f32 = 2.0;

/// A pulse of light fired by a kick (or the ambient clock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    /// Track seconds on this state's clock.
    pub born: f64,
    /// Which pulse this is, counted from the start: hashes to its vertex.
    pub id: u64,
    pub strength: f32,
}

/// The lyric line the scene is setting, kept after its cue ends so it can
/// dissolve rather than vanish.
#[derive(Clone, Debug, PartialEq)]
pub struct LineMemory {
    pub id: u64,
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlyphLatticeState {
    seed: u64,
    clock: f64,
    /// Track-relative loudness, bass, mids, treble; each already through
    /// `response`, with loudness pulled toward a neutral 0.5 as it falls.
    music: [f32; 4],
    bands: [f32; BAND_GROUPS],
    /// Kick envelope: a brief scale-up and bloom.
    punch: f32,
    /// Kick spin impulse, radians per second, decaying.
    lurch: f32,
    kick: KickDetector,
    pulses: Vec<Pulse>,
    fired: u64,
    since_pulse: f32,
    onset_cooldown: f32,
    onset_held: bool,
    /// The same adaptive detector as the kick's, on the top quarter of the
    /// bands: the hats. Real mixes rarely reach the absolute onset lane, and
    /// a ripple that fires twice a minute is a ripple nobody sees.
    hats: KickDetector,
    /// Birth time and strength of each live ripple.
    ripples: Vec<(f64, f32)>,
    ripple_count: u64,
    /// Yaw, drift phase, fourth-axis angle; f64 so a long track never loses
    /// the small per-frame increments.
    yaw: f64,
    drift: f64,
    fourth: f64,
    history: [[f32; 3]; HISTORY],
    head: usize,
    written: u64,
    form: Form,
    form_since: f64,
    previous: Option<Form>,
    previous_since: f64,
    /// The automatic cycle's order, permuted by the seed.
    cycle: [Form; 6],
    cycle_index: usize,
    quiet: f32,
    line: Option<LineMemory>,
    line_live: bool,
    presence: f32,
}

fn mix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl GlyphLatticeState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut cycle = Form::ALL;
        // The cube always opens: it is the silhouette the scene is known by.
        // The rest follow in a seeded order.
        let mut state = mix64(seed);
        for i in (2..cycle.len()).rev() {
            state = mix64(state);
            let j = 1 + (state % i as u64) as usize;
            cycle.swap(i, j);
        }
        let yaw = (mix64(seed ^ 0x5151) >> 40) as f64 / 16_777_216.0 * 0.6;
        Self {
            seed,
            clock: 0.0,
            music: [0.5, 0.0, 0.0, 0.0],
            bands: [0.0; BAND_GROUPS],
            punch: 0.0,
            lurch: 0.0,
            kick: KickDetector::new(),
            pulses: Vec::new(),
            fired: 0,
            since_pulse: 0.0,
            onset_cooldown: 0.0,
            onset_held: false,
            hats: KickDetector::new(),
            ripples: Vec::new(),
            ripple_count: 0,
            yaw,
            drift: 0.0,
            fourth: 0.0,
            history: [[yaw as f32, 0.0, 0.0]; HISTORY],
            head: 0,
            written: 0,
            form: cycle[0],
            form_since: 0.0,
            previous: None,
            previous_since: -FADE_SECONDS,
            cycle,
            cycle_index: 0,
            quiet: 0.0,
            line: None,
            line_live: false,
            presence: 0.0,
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub fn clock(&self) -> f64 {
        self.clock
    }

    #[must_use]
    pub fn form(&self) -> Form {
        self.form
    }

    #[must_use]
    pub fn previous(&self) -> Option<Form> {
        self.previous
    }

    /// Seconds since the current solid began to assemble.
    #[must_use]
    pub fn form_age(&self) -> f64 {
        self.clock - self.form_since
    }

    /// Seconds since the previous solid began to exhale.
    #[must_use]
    pub fn previous_age(&self) -> f64 {
        self.clock - self.previous_since
    }

    /// Loudness, bass, mids, treble.
    #[must_use]
    pub fn music(&self) -> [f32; 4] {
        self.music
    }

    #[must_use]
    pub fn bands(&self) -> &[f32; BAND_GROUPS] {
        &self.bands
    }

    #[must_use]
    pub fn punch(&self) -> f32 {
        self.punch
    }

    #[must_use]
    pub fn pulses(&self) -> &[Pulse] {
        &self.pulses
    }

    /// Pulses fired since the start, by kicks and by the ambient clock.
    #[must_use]
    pub fn fired(&self) -> u64 {
        self.fired
    }

    /// Kicks the detector has heard.
    #[must_use]
    pub fn kicks(&self) -> u64 {
        self.kick.count()
    }

    #[must_use]
    pub fn ripples(&self) -> &[(f64, f32)] {
        &self.ripples
    }

    #[must_use]
    pub fn ripple_count(&self) -> u64 {
        self.ripple_count
    }

    /// Yaw and the slow drift phase the pitch, roll and the giants ride.
    #[must_use]
    pub fn angles(&self) -> (f32, f32, f32) {
        (
            self.yaw.rem_euclid(TAU) as f32,
            // Never wrapped: the pose reads them at incommensurate rates.
            self.drift as f32,
            self.fourth as f32,
        )
    }

    /// The (yaw, drift, fourth) the solid had `age` seconds ago, for its
    /// echoes. Older than the history reaches returns the oldest slot.
    #[must_use]
    pub fn angles_ago(&self, age: f32) -> (f32, f32, f32) {
        let slots = (f64::from(age.max(0.0)) / HISTORY_SLOT).round() as usize;
        if slots == 0 {
            return self.angles();
        }
        let slots = slots.min(HISTORY - 1);
        let slot = self.history[(self.head + HISTORY - slots) % HISTORY];
        (slot[0], slot[1], slot[2])
    }

    #[must_use]
    pub fn line(&self) -> Option<&LineMemory> {
        self.line.as_ref()
    }

    /// Whether the remembered line's cue is the one playing.
    #[must_use]
    pub fn line_live(&self) -> bool {
        self.line_live
    }

    /// How far the scene has handed the frame to the line, `0..=1`.
    #[must_use]
    pub fn presence(&self) -> f32 {
        self.presence
    }

    /// The automatic cycle's order, for the report and the tests.
    #[must_use]
    pub fn cycle(&self) -> [Form; 6] {
        self.cycle
    }

    fn switch(&mut self, form: Form) {
        if form == self.form {
            return;
        }
        self.previous = Some(self.form);
        self.previous_since = self.clock;
        self.form = form;
        self.form_since = self.clock;
        self.quiet = 0.0;
    }

    fn fire(&mut self, strength: f32) {
        self.fired += 1;
        self.since_pulse = 0.0;
        if self.pulses.len() >= MAX_PULSES {
            self.pulses.remove(0);
        }
        self.pulses.push(Pulse {
            born: self.clock,
            id: self.fired,
            strength: strength.clamp(0.0, 1.0),
        });
    }

    fn fold_bands(&mut self, bands: &[f32], response: f32, dt: f32) {
        for (group, value) in self.bands.iter_mut().enumerate() {
            let target = if bands.is_empty() {
                0.0
            } else {
                let start = group * bands.len() / BAND_GROUPS;
                let end = ((group + 1) * bands.len() / BAND_GROUPS)
                    .max(start + 1)
                    .min(bands.len());
                let mean = bands[start..end].iter().copied().sum::<f32>() / (end - start) as f32;
                // Analyzer bands sit low and the upper ones lower still; the
                // tilt lets the treble edges light as readily as the bass.
                let tilt = 1.5 + 1.8 * group as f32 / (BAND_GROUPS - 1) as f32;
                let level = if mean.is_finite() { mean } else { 0.0 };
                ((level * tilt).clamp(0.0, 1.0) * response).clamp(0.0, 1.0)
            };
            follow(value, target, 14.0, 4.0, dt);
        }
    }
}

/// The hats' transient: the mean positive excursion of the top quarter of
/// the bands over their own trails.
fn treble_excess(audio: &crate::scene::SceneAudioFrame<'_>) -> f32 {
    let count = audio.bands.len().min(audio.trails.len());
    if count == 0 {
        return 0.0;
    }
    let from = count - (count / 4).max(1);
    let excess = audio.bands[from..count]
        .iter()
        .zip(&audio.trails[from..count])
        .map(|(band, trail)| (band - trail).max(0.0))
        .sum::<f32>()
        / (count - from) as f32;
    if excess.is_finite() {
        excess
    } else {
        0.0
    }
}

impl SceneState for GlyphLatticeState {
    fn id(&self) -> SceneId {
        SceneId::GlyphLattice
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        if !frame.delta_seconds.is_finite() || frame.delta_seconds <= 0.0 {
            return;
        }
        let dt = frame.delta_seconds.min(0.25);
        let id = self.id();
        let response = frame.setting(id, control::RESPONSE).clamp(0.0, 2.0);
        let spin = frame.setting(id, control::SPIN).max(0.0);
        let words = frame.setting(id, control::WORDS) > 0.5;
        let features = Features::of(&frame.audio, frame.dynamics);

        // Loudness leans toward a neutral middle as the response falls, so
        // `response` 0 is a steady mid-weight solid rather than the thin one
        // silence would draw.
        let level = (0.5 + (features.level - 0.5) * response).clamp(0.0, 1.0);
        let bass = ((features.bass - 0.14) / 0.5).clamp(0.0, 1.0) * response;
        let targets = [
            level,
            bass.clamp(0.0, 1.0),
            ((features.mids * 1.6).clamp(0.0, 1.0) * response).clamp(0.0, 1.0),
            ((features.treble * 2.4).clamp(0.0, 1.0) * response).clamp(0.0, 1.0),
        ];
        let rates = [(3.0, 0.9), (9.0, 2.4), (6.0, 2.0), (12.0, 3.5)];
        for ((current, target), (attack, release)) in self.music.iter_mut().zip(targets).zip(rates)
        {
            follow(current, target, attack, release, dt);
        }
        self.fold_bands(frame.audio.bands, response, dt);

        self.punch *= (-dt * 6.0).exp();
        self.lurch *= (-dt * 5.5).exp();
        self.since_pulse += dt;
        let kick = self.kick.step(features.kick * response, dt);
        let mut beat = false;
        if let Some(strength) = kick {
            self.punch = self.punch.max(strength);
            // About five degrees for a full-strength kick, spread over ~0.2 s:
            // a tick on the beat, not a shove.
            self.lurch += 0.5 * strength * spin.min(1.5);
            self.fire(strength);
            if strength > 0.78 {
                // A hard kick lights a second vertex at once.
                self.fire(strength * 0.8);
            }
            beat = true;
        } else if self.since_pulse > AMBIENT_GAP {
            self.fire(0.45);
            beat = true;
        }
        self.pulses
            .retain(|pulse| self.clock - pulse.born < 2.6 && pulse.strength > 0.0);

        // Ripples: a hat tick or an onset's rising edge, at most one every
        // half second, so the texture turns over in waves rather than boils.
        self.onset_cooldown = (self.onset_cooldown - dt).max(0.0);
        let hat = self.hats.step(treble_excess(&frame.audio) * response, dt);
        let onset = frame.audio.onset && response > 0.0;
        let rising = onset && !self.onset_held;
        if (rising || hat.is_some()) && self.onset_cooldown <= 0.0 {
            self.onset_cooldown = 0.5;
            self.ripple_count += 1;
            if self.ripples.len() >= RIPPLES {
                self.ripples.remove(0);
            }
            let strength = hat
                .unwrap_or(0.0)
                .max((0.35 + features.flux * 5.0).clamp(0.35, 1.0) * f32::from(u8::from(rising)))
                .max(0.4)
                * response.min(1.0);
            self.ripples.push((self.clock, strength));
        }
        self.onset_held = onset;
        self.ripples
            .retain(|(born, _)| self.clock - born < RIPPLE_SECONDS);

        // Spin: quiet passages turn slowly, loud ones faster; kicks lurch.
        // A slow turn: about 4 degrees a second when quiet, 17 when loud.
        let pace = f64::from(spin) * (0.07 + 0.23 * f64::from(self.music[0]));
        self.yaw += f64::from(dt) * (pace + f64::from(self.lurch));
        self.drift += f64::from(dt) * f64::from(spin) * (0.08 + 0.12 * f64::from(self.music[0]));
        self.fourth += f64::from(dt)
            * (f64::from(spin) * (0.14 + 0.3 * f64::from(self.music[0]))
                + f64::from(self.lurch) * 1.6);

        // Solids: a setting names one; 0 runs the cycle.
        match Form::from_setting(frame.setting(id, control::SOLID)) {
            Some(form) => self.switch(form),
            None => {
                if self.music[0] < 0.3 {
                    self.quiet += dt;
                }
                let held = self.clock - self.form_since;
                let drop = beat && self.music[0] > 0.6 && self.quiet > DROP_QUIET;
                let moved_on = beat && held > AUTO_HOLD;
                if (drop || moved_on) && held > 4.0 {
                    self.cycle_index = (self.cycle_index + 1) % self.cycle.len();
                    let next = self.cycle[self.cycle_index];
                    self.switch(next);
                }
                if self.music[0] > 0.45 && !drop {
                    self.quiet = (self.quiet - dt * 0.5).max(0.0);
                }
            }
        }

        // The lyric line: remembered while it plays and while it dissolves.
        match frame.lyric.filter(|_| words) {
            Some(cue) if !cue.text.trim().is_empty() => {
                if self.line.as_ref().is_none_or(|line| line.id != cue.id) {
                    self.line = Some(LineMemory {
                        id: cue.id,
                        text: cue.text.clone(),
                        start: cue.start_seconds,
                        end: cue.end_seconds,
                    });
                }
                self.line_live = true;
            }
            _ => self.line_live = false,
        }
        follow(
            &mut self.presence,
            if self.line_live { 1.0 } else { 0.0 },
            7.0,
            2.4,
            dt,
        );
        if !self.line_live && self.presence < 0.004 {
            self.line = None;
        }

        // Angle history in track time, so echoes trail by the same amount in
        // a 30 fps export as in the 60 Hz preview.
        self.clock += f64::from(dt);
        let due = (self.clock / HISTORY_SLOT + 1.0e-6).floor() as u64;
        while self.written < due {
            self.written += 1;
            self.head = (self.head + 1) % HISTORY;
            let (yaw, drift, fourth) = self.angles();
            self.history[self.head] = [yaw, drift, fourth];
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::GlyphLattice,
        state_version: 1,
        make_state: |seed| Box::new(GlyphLatticeState::new(seed)),
    }
}

#[cfg(test)]
mod tests;
