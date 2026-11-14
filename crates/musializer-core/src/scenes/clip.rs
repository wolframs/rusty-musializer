//! Video Clip: a video file as a section's picture (post-legacy, 2026-11-14).
//!
//! The deterministic half. Which of the clip's frames is on screen is not
//! state at all — it is [`crate::project::clip::ClipMoment::frame`], a pure
//! function of the frame's time — so this holds only what the music does to
//! the picture, and hands the drawing half a [`ClipLook`]:
//!
//! - **punch**: each kick zooms in a few percent and lets go over a fifth of
//!   a second, so the cut of the clip lands on the beat;
//! - **pulse**: the kick lifts the exposure and the loudness swells it, a
//!   light on the picture rather than a flash;
//! - **drift**: a slow push in and sideways, as if the camera breathed, faster
//!   when the track is loud;
//! - **shake**: the hardest hits jolt the frame sideways, off by default;
//! - **tint**: the highlights warm with the bass and cool with the treble.
//!
//! Defaults are low: the clip is somebody's picture, and the music should touch
//! it, not wreck it. `response` 0 removes every audio term — the drift keeps
//! its own clock — and the look is then the same number for any audio, which
//! the tests pin. Envelopes advance on fixed 60 Hz substeps like Fluid Ink's,
//! so a 30 fps export and a 60 Hz preview see the same envelope.

use crate::scene::settings::index::clip as control;
use crate::scene::{SceneAudioFrame, SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;

const STEP: f32 = 1.0 / 60.0;
/// At most six substeps per update, as Fluid Ink: a stalled preview frame must
/// not make the next one stall.
const MAX_BANKED: f32 = 6.0 * STEP;
/// Clawd's and Fluid Ink's kick detector: the mean rise of the lowest quarter
/// of the bands over their trails.
const KICK_THRESHOLD: f32 = 0.05;
const KICK_REFRACTORY: f32 = 0.2;
/// A kick past this many thresholds also shakes.
const SHAKE_KICK: f32 = 2.4;
const PUNCH_HALF_LIFE: f32 = 0.13;
const PULSE_HALF_LIFE: f32 = 0.22;
const SHAKE_HALF_LIFE: f32 = 0.07;
/// The most a full punch zooms in, at `punch` 1 and `response` 1.
const PUNCH_ZOOM: f32 = 0.085;
/// The most a full pulse lifts the exposure.
const PULSE_GAIN: f32 = 0.55;
/// The drift's widest push in and sway, as fractions of the frame.
const DRIFT_ZOOM: f32 = 0.07;
const DRIFT_SWAY: f32 = 0.025;
/// A full shake, as a fraction of the frame's width.
const SHAKE_REACH: f32 = 0.018;

/// What the music does to the picture this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipLook {
    /// `>= 1`: how far in the picture is, about the frame's centre.
    pub zoom: f32,
    /// The picture's offset, as fractions of the frame (x right, y down).
    pub offset: [f32; 2],
    /// Multiplies the picture's light; 1 is the clip as it is.
    pub exposure: f32,
    /// `-1` cool to `1` warm, already scaled by `tint`: how far the
    /// highlights lean.
    pub tint: f32,
}

impl ClipLook {
    pub const NEUTRAL: Self = Self {
        zoom: 1.0,
        offset: [0.0, 0.0],
        exposure: 1.0,
        tint: 0.0,
    };
}

/// The envelopes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipMusic {
    pub punch: f32,
    pub pulse: f32,
    pub shake: f32,
    /// Track-relative loudness, a slow follower.
    pub level: f32,
    /// Treble's share against bass, `-1` bass-heavy to `1` treble-heavy.
    pub balance: f32,
    pub kicks: u64,
    pub shakes: u64,
}

pub struct ClipState {
    banked: f32,
    refractory: f32,
    /// The drift's clock, in drift-seconds.
    drift: f64,
    /// Which way the next shake goes.
    shake_side: f32,
    music: ClipMusic,
    look: ClipLook,
}

impl ClipState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            banked: 0.0,
            refractory: 0.0,
            // Seeded, so two clip sections of one video do not drift alike.
            drift: (seed % 1013) as f64 * 0.37,
            shake_side: if seed.is_multiple_of(2) { 1.0 } else { -1.0 },
            music: ClipMusic::default(),
            look: ClipLook::NEUTRAL,
        }
    }

    #[must_use]
    pub fn look(&self) -> ClipLook {
        self.look
    }

    #[must_use]
    pub fn music(&self) -> ClipMusic {
        self.music
    }

    fn step(&mut self, frame: &SceneFrame<'_>, response: f32) {
        let audio = &frame.audio;
        let loud = frame
            .dynamics
            .map_or(audio.rms * 1.8, |profile| profile.level(audio.rms))
            .clamp(0.0, 1.0);
        let m = &mut self.music;
        m.level = follow(
            m.level,
            if loud.is_finite() { loud } else { 0.0 },
            0.08,
            0.9,
        );
        let (bass, _, treble) = audio.spectral_regions();
        let balance = if bass + treble > 0.02 {
            ((treble - bass) / (bass + treble)).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        m.balance = follow(m.balance, balance, 0.5, 0.5);
        m.punch *= decay(PUNCH_HALF_LIFE);
        m.pulse *= decay(PULSE_HALF_LIFE);
        m.shake *= decay(SHAKE_HALF_LIFE);
        self.refractory = (self.refractory - STEP).max(0.0);
        let kick = raw_kick(audio);
        if kick >= KICK_THRESHOLD && self.refractory <= 0.0 {
            self.refractory = KICK_REFRACTORY;
            let strength = (kick / (3.0 * KICK_THRESHOLD)).clamp(0.4, 1.0);
            m.punch = m.punch.max(strength);
            m.pulse = m.pulse.max(strength);
            m.kicks += 1;
            if kick >= SHAKE_KICK * KICK_THRESHOLD {
                m.shake = 1.0;
                m.shakes += 1;
                self.shake_side = -self.shake_side;
            }
        }
        // The drift's clock runs whatever the music does; loudness only
        // hurries it, and only through `response`.
        let speed = 0.06 + 0.12 * f64::from(m.level * response);
        self.drift += f64::from(STEP) * speed;
    }

    fn compose(&self, frame: &SceneFrame<'_>) -> ClipLook {
        let id = SceneId::Clip;
        let response = frame.setting(id, control::RESPONSE);
        let punch = frame.setting(id, control::PUNCH) * response;
        let pulse = frame.setting(id, control::PULSE) * response;
        let shake = frame.setting(id, control::SHAKE) * response;
        let tint = frame.setting(id, control::TINT) * response;
        let drift = frame.setting(id, control::DRIFT);
        let m = &self.music;
        let phase = self.drift;
        let breathe = (0.5 + 0.5 * (phase * 0.9).sin()) as f32;
        let zoom = 1.0 + DRIFT_ZOOM * drift * breathe + PUNCH_ZOOM * punch * m.punch;
        let sway = [
            DRIFT_SWAY * drift * (phase * 0.61).sin() as f32,
            0.6 * DRIFT_SWAY * drift * (phase * 0.43 + 1.3).sin() as f32,
        ];
        let jolt = SHAKE_REACH * shake * m.shake * self.shake_side;
        ClipLook {
            zoom,
            offset: [sway[0] + jolt, sway[1]],
            exposure: 1.0
                + PULSE_GAIN * pulse * (0.7 * m.pulse + 0.3 * m.level * m.pulse.max(0.25)),
            tint: (-m.balance * tint).clamp(-1.0, 1.0),
        }
    }
}

impl SceneState for ClipState {
    fn id(&self) -> SceneId {
        SceneId::Clip
    }

    fn update(&mut self, frame: &SceneFrame<'_>) {
        let response = frame.setting(SceneId::Clip, control::RESPONSE);
        self.banked = (self.banked + frame.delta_seconds.max(0.0)).min(MAX_BANKED);
        while self.banked >= STEP - 1.0e-6 {
            self.banked -= STEP;
            self.step(frame, response);
        }
        self.look = self.compose(frame);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn decay(half_life: f32) -> f32 {
    (-STEP * std::f32::consts::LN_2 / half_life).exp()
}

/// A one-pole follower with separate rise and fall times, in seconds.
fn follow(current: f32, target: f32, rise: f32, fall: f32) -> f32 {
    let time = if target > current { rise } else { fall };
    current + (target - current) * (1.0 - (-STEP / time).exp())
}

fn raw_kick(audio: &SceneAudioFrame<'_>) -> f32 {
    let count = (audio.bands.len() / 4).max(1).min(audio.bands.len());
    if count == 0 || audio.trails.len() < count {
        return 0.0;
    }
    let rise: f32 = audio.bands[..count]
        .iter()
        .zip(&audio.trails[..count])
        .map(|(band, trail)| (band - trail).max(0.0))
        .sum();
    let rise = rise / count as f32;
    if rise.is_finite() {
        rise
    } else {
        0.0
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::Clip,
        state_version: 1,
        make_state: |seed| Box::new(ClipState::new(seed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    fn run(settings: &SceneSettings, loud: bool, frames: usize) -> Vec<ClipLook> {
        let mut state = ClipState::new(7);
        let quiet = [0.0f32; 16];
        let mut looks = Vec::with_capacity(frames);
        for index in 0..frames {
            // A kick every half second: the low bands jump over their trails.
            let kick = loud && index % 15 == 0;
            let bands: Vec<f32> = (0..16)
                .map(|band| {
                    if kick && band < 4 {
                        0.9
                    } else if loud {
                        0.3
                    } else {
                        0.0
                    }
                })
                .collect();
            let trails: Vec<f32> = (0..16).map(|_| if loud { 0.25 } else { 0.0 }).collect();
            let mut frame = SceneFrame::idle(settings);
            frame.delta_seconds = 1.0 / 30.0;
            frame.time_seconds = index as f64 / 30.0;
            frame.audio.rms = if loud { 0.6 } else { 0.0 };
            if loud {
                frame.audio.bands = &bands;
                frame.audio.trails = &trails;
            } else {
                frame.audio.bands = &quiet;
                frame.audio.trails = &quiet;
            }
            state.update(&frame);
            looks.push(state.look());
        }
        looks
    }

    #[test]
    fn the_kicks_punch_the_picture_and_quiet_leaves_it_drifting() {
        let settings = SceneSettings::new();
        let loud = run(&settings, true, 90);
        let quiet = run(&settings, false, 90);
        let max_zoom = |looks: &[ClipLook]| looks.iter().map(|l| l.zoom).fold(0.0, f32::max);
        let max_light = |looks: &[ClipLook]| looks.iter().map(|l| l.exposure).fold(0.0, f32::max);
        assert!(max_zoom(&loud) > max_zoom(&quiet) + 0.01, "punch");
        assert!(max_light(&loud) > 1.05, "pulse");
        assert_eq!(max_light(&quiet), 1.0, "no light without music");
        // The drift alone stays inside its bounds.
        assert!(quiet
            .iter()
            .all(|l| l.zoom >= 1.0 && l.zoom <= 1.0 + DRIFT_ZOOM));
    }

    #[test]
    fn response_zero_is_the_same_look_for_any_audio() {
        let mut settings = SceneSettings::new();
        settings.set(SceneId::Clip, control::RESPONSE, 0.0);
        settings.set(SceneId::Clip, control::SHAKE, 1.0);
        let loud = run(&settings, true, 120);
        let quiet = run(&settings, false, 120);
        assert_eq!(loud, quiet);
    }

    #[test]
    fn everything_at_zero_is_the_clip_as_it_is() {
        let mut settings = SceneSettings::new();
        for index in [
            control::PUNCH,
            control::PULSE,
            control::DRIFT,
            control::SHAKE,
            control::TINT,
        ] {
            settings.set(SceneId::Clip, index, 0.0);
        }
        for look in run(&settings, true, 60) {
            assert_eq!(look, ClipLook::NEUTRAL);
        }
    }
}
