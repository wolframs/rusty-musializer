//! Lumen Knot (HY-6, 2026-11-17): one luminous strand tied as a torus knot.
//!
//! A torus knot T(p, q) winds `p` times round a ring while it swings `q` times
//! through it, so seen along its axis it is a rosette of `q` petals woven over
//! and under itself. The swing's depth is flattened to half its radial reach
//! ([`DEPTH`]): deep enough that every crossing clears the tube, flat enough
//! that the inner turns do not dive end-on at the viewer, which read as a cut
//! strand.
//!
//! Six forms are tabled in [`FORMS`]; the `form` setting picks one per section
//! and 0 lets the song move on at a drop or after a long hold. A change pulls
//! the knot tight into a braided ring and lets it bloom out as the next form:
//! at the tightest point every form is nearly the same ring, which is what
//! hides the change of knot. Music swells the petals by angle (never the
//! crossing order or the opening), and kicks send comets of light round the
//! strand, over and under itself.
//!
//! Until 2026-11-17 this scene was six fixed hooked ribbons (SX5);
//! `docs/LUMEN_KNOT.md` says why it changed.
use super::aurora_veil::{follow, Features, KickDetector};
use crate::scene::settings::index::knot as control;
use crate::scene::{SceneDescriptor, SceneFrame, SceneId, SceneState};
use std::any::Any;
use std::f32::consts::TAU;

/// Six spectral regions, one per sixth of the knot's angle.
pub const BANDS: usize = 6;
/// Samples along the strand. Constant across forms, so the drawn triangle
/// count is a fixed number a report can pin.
pub const SEGMENTS: usize = 960;
/// The swing's depth as a share of its radial reach.
pub const DEPTH: f32 = 0.5;
/// The tightest a change pulls the swing, as a share of the form's own.
pub const PINCH_FLOOR: f32 = 0.05;
/// Seconds from one form, through the ring, to the next.
pub const MORPH_SECONDS: f32 = 1.8;
/// Share of a change spent pulling tight. The pinch is quick so it lands on
/// the hit that caused it (a drop, a cut); the bloom into the new form rides
/// what follows.
pub const PINCH_SHARE: f32 = 0.25;
/// A comet's head travels this share of the strand per second at Light
/// travel 1.
pub const COMET_SPEED: f32 = 0.16;
/// Seconds a comet lives.
pub const COMET_SECONDS: f32 = 2.6;
/// Live comets at most; a kick past this replaces the oldest.
pub const COMETS: usize = 8;
/// Without a kick this long, the strand sends a soft comet of its own, so a
/// quiet passage and Strand response 0 still have light moving.
const AMBIENT_GAP: f32 = 3.2;
/// Seconds of quiet before a kick counts as a drop. With the follower's fall
/// this recognises a break of about two and a half seconds.
const DROP_QUIET: f32 = 1.5;
/// The automatic cycle moves on after this long on one form, at a kick.
const AUTO_HOLD: f32 = 20.0;
/// And never sooner than this, however the song drops.
const MIN_HOLD: f32 = 6.0;
/// Radial reach of the strongest breathing: `1 + BREATH * 2`.
pub const BREATH: f32 = 0.075;
/// The largest radius any form reaches, tube included, before breathing.
pub const EXTENT: f32 = 0.98;

/// One knot the scene can tie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Form {
    pub name: &'static str,
    /// Times round the ring (`p`).
    pub winds: u32,
    /// Times through it, which is the petal count (`q`).
    pub petals: u32,
    /// The ring's radius, in units of the frame's fit.
    pub ring: f32,
    /// How far the strand swings out from the ring and back in.
    pub swing: f32,
    /// The tube's radius. Denser forms get a thinner tube so the weave stays
    /// open at a phone's width.
    pub tube: f32,
}

/// A persisted index (setting value - 1): append, never reorder. The first is
/// the six-petal rosette the scene opens on.
#[rustfmt::skip]
pub const FORMS: [Form; 6] = [
    Form { name: "six-petal rosette",   winds: 5, petals: 6, ring: 0.58, swing: 0.30, tube: 0.042 },
    Form { name: "seven-petal rosette", winds: 3, petals: 7, ring: 0.60, swing: 0.31, tube: 0.050 },
    Form { name: "five-petal rosette",  winds: 3, petals: 5, ring: 0.60, swing: 0.31, tube: 0.055 },
    Form { name: "eight-petal rosette", winds: 3, petals: 8, ring: 0.60, swing: 0.31, tube: 0.046 },
    Form { name: "five-point star",     winds: 2, petals: 5, ring: 0.60, swing: 0.32, tube: 0.060 },
    Form { name: "seven-point star",    winds: 2, petals: 7, ring: 0.61, swing: 0.31, tube: 0.056 },
];

/// `form` 1..=6 names one; 0 (or anything unreadable) lets the song choose.
#[must_use]
pub fn form_from_setting(value: f32) -> Option<usize> {
    if !value.is_finite() {
        return None;
    }
    let index = value.round();
    (index >= 1.0 && index <= FORMS.len() as f32).then(|| index as usize - 1)
}

/// What the renderer ties this frame: a form, its swing, and the ring and
/// tube it is easing between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub form: usize,
    /// Share of the form's swing: 1 settled, [`PINCH_FLOOR`] at a change's
    /// tightest point.
    pub swing: f32,
    pub ring: f32,
    pub tube: f32,
    /// A bell over a change, peaking as the forms trade places.
    pub flare: f32,
}

/// A point on the strand at `t` (0..1, closed), in the knot's own frame: `z`
/// is depth toward the viewer before the tilt.
#[must_use]
pub fn point(pose: &Pose, t: f32) -> [f32; 3] {
    let form = &FORMS[pose.form.min(FORMS.len() - 1)];
    let angle = t * TAU;
    let swing = form.swing * pose.swing;
    let (sq, cq) = (form.petals as f32 * angle).sin_cos();
    let (sp, cp) = (form.winds as f32 * angle).sin_cos();
    let radius = pose.ring + swing * cq;
    [radius * cp, radius * sp, DEPTH * swing * sq]
}

/// The settled pose of one form.
#[must_use]
pub fn settled(form: usize) -> Pose {
    let f = &FORMS[form.min(FORMS.len() - 1)];
    Pose {
        form,
        swing: 1.0,
        ring: f.ring,
        tube: f.tube,
        flare: 0.0,
    }
}

/// A shared radial swell by angle, applied to every strand point in the
/// knot's own frame. Its factor is positive and constant along each ray, so
/// the plane maps onto itself one to one: crossings stay crossings, depth is
/// untouched, and the opening cannot close.
#[must_use]
pub fn breathe(p: [f32; 3], bands: &[f32; BANDS], response: f32) -> [f32; 3] {
    let angle = p[1].atan2(p[0]);
    let mut weight = 0.0;
    let mut drive = 0.0;
    for (i, band) in bands.iter().enumerate() {
        let w = (0.5 + 0.5 * (angle - i as f32 * TAU / BANDS as f32).cos()).powi(6);
        weight += w;
        drive += w * unit(*band);
    }
    let scale = 1.0 + BREATH * response.clamp(0.0, 2.0) * drive / weight.max(0.001);
    [p[0] * scale, p[1] * scale, p[2]]
}

fn unit(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A pulse of light travelling the strand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comet {
    /// Head position along the strand, 0..1 and wrapping.
    pub head: f32,
    /// Scene seconds since it was sent.
    pub age: f32,
    pub strength: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LumenKnotState {
    pub bands: [f32; BANDS],
    pub energy: f32,
    pub beat: f32,
    /// Kick envelope, 0..1: the flash and lift a kick gives the whole strand.
    pub punch: f32,
    pub time: f32,
    pub phase: f32,
    /// The form shown, or bloomed into during a change, and the one left.
    pub form: usize,
    pub from: usize,
    /// 0 at a change's start, 1 once settled.
    pub morph: f32,
    /// Seconds on the current form, and of quiet (for a drop).
    held: f32,
    quiet: f32,
    /// Loudness, rising fast and falling over about two-thirds of a second.
    loud: f32,
    /// Spin about the knot's own axis, radians. f64 so a long track keeps the
    /// small per-frame increments.
    pub spin: f64,
    lurch: f32,
    kick: KickDetector,
    pub comets: Vec<Comet>,
    /// Comets sent since the start; hashes each one's starting point.
    pub fired: u64,
    since_comet: f32,
    started: bool,
}

impl LumenKnotState {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            bands: [0.0; BANDS],
            energy: 0.0,
            beat: 0.0,
            punch: 0.0,
            time: 0.0,
            phase: (seed % 65536) as f32 / 65536.0 * TAU,
            form: 0,
            from: 0,
            morph: 1.0,
            held: 0.0,
            quiet: 0.0,
            loud: 0.0,
            spin: 0.0,
            lurch: 0.0,
            kick: KickDetector::new(),
            comets: Vec::new(),
            fired: 0,
            since_comet: 0.0,
            started: false,
        }
    }

    /// What to draw this frame.
    #[must_use]
    pub fn pose(&self) -> Pose {
        if self.morph >= 1.0 {
            return settled(self.form);
        }
        let m = self.morph.clamp(0.0, 1.0);
        let s = if m < PINCH_SHARE {
            0.5 * m / PINCH_SHARE
        } else {
            0.5 + 0.5 * (m - PINCH_SHARE) / (1.0 - PINCH_SHARE)
        };
        let (a, b) = (&FORMS[self.from], &FORMS[self.form]);
        let blend = smoothstep(s);
        Pose {
            form: if s < 0.5 { self.from } else { self.form },
            swing: smoothstep((1.0 - 2.0 * s).abs()).max(PINCH_FLOOR),
            ring: a.ring + (b.ring - a.ring) * blend,
            tube: a.tube + (b.tube - a.tube) * blend,
            flare: (-((s - 0.5) / 0.2).powi(2)).exp(),
        }
    }

    fn start(&mut self, form: usize) {
        self.from = self.form;
        self.form = form;
        self.morph = 0.0;
        self.held = 0.0;
    }

    fn fire(&mut self, strength: f32) {
        self.fired += 1;
        if self.comets.len() >= COMETS {
            self.comets.remove(0);
        }
        // A golden-ratio walk spreads successive comets round the strand; the
        // seed's phase moves where the walk starts.
        let head = (self.fired as f32 * 0.618_034 + self.phase / TAU).fract();
        self.comets.push(Comet {
            head,
            age: 0.0,
            strength: unit(strength),
        });
        self.since_comet = 0.0;
    }
}

impl SceneState for LumenKnotState {
    fn id(&self) -> SceneId {
        SceneId::LumenKnot
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn update(&mut self, frame: &SceneFrame<'_>) {
        let id = SceneId::LumenKnot;
        // Absolute scene time makes seeks and a frozen frame stable. Followers
        // are replayed by the existing deterministic preview/export machinery.
        self.time = if frame.time_seconds.is_finite() {
            frame.time_seconds.max(0.0) as f32
        } else {
            0.0
        };
        // A fresh state starts on its form, already tied: a section that opens
        // on this scene must not open on a change.
        if !self.started {
            self.started = true;
            self.form = form_from_setting(frame.setting(id, control::FORM)).unwrap_or(0);
            self.from = self.form;
        }
        let dt = if frame.delta_seconds.is_finite() {
            frame.delta_seconds.clamp(0.0, 0.1)
        } else {
            0.0
        };
        if dt <= 0.0 {
            return;
        }
        let n = frame.audio.bands.len();
        for (i, out) in self.bands.iter_mut().enumerate() {
            let start = n * i / BANDS;
            let end = n * (i + 1) / BANDS;
            let target = if end > start {
                (frame.audio.bands[start..end]
                    .iter()
                    .map(|x| unit(*x).powi(2))
                    .sum::<f32>()
                    / (end - start) as f32)
                    .sqrt()
            } else {
                0.0
            };
            let rate = if target > *out { 18.0 } else { 4.0 };
            *out += (target - *out) * (1.0 - (-dt * rate).exp());
        }
        self.energy = self.bands.iter().sum::<f32>() / BANDS as f32;
        self.beat *= (-dt * 5.0).exp();
        if frame.audio.onset {
            self.beat = self.beat.max(unit(frame.audio.spectral_flux * 4.0));
        }

        let response = frame.setting(id, control::RESPONSE);
        let features = Features::of(&frame.audio, frame.dynamics);
        let kick = self.kick.step(features.kick * response, dt);
        self.since_comet += dt;
        self.punch *= (-dt * 5.0).exp();
        if let Some(strength) = kick {
            self.fire(strength);
            self.punch = self.punch.max(unit(strength));
            // A tick of spin on the beat, not a shove: about four degrees.
            self.lurch += 0.45 * strength * frame.setting(id, control::PULSE).min(1.5);
        } else if self.since_comet > AMBIENT_GAP {
            self.fire(0.4);
        }
        let travel = dt * COMET_SPEED * frame.setting(id, control::FLOW);
        for comet in &mut self.comets {
            comet.age += dt;
            comet.head = (comet.head + travel).rem_euclid(1.0);
        }
        self.comets.retain(|comet| comet.age < COMET_SECONDS);

        // Quiet is judged on a loudness follower that falls slower than the
        // gap between kicks: on the instantaneous level, the dips between the
        // kicks of a loud loop added up to "quiet" and fired a false drop
        // every few seconds.
        follow(
            &mut self.loud,
            features.level * response.min(1.0),
            8.0,
            1.5,
            dt,
        );
        if self.loud < 0.2 {
            self.quiet += dt;
        } else if self.loud > 0.35 {
            self.quiet = 0.0;
        }

        // Forms: a setting names one; 0 lets the song move on.
        self.held += dt;
        match form_from_setting(frame.setting(id, control::FORM)) {
            Some(form) => {
                if form != self.form && self.morph >= 1.0 {
                    self.start(form);
                }
            }
            None => {
                // A kick out of real quiet is a drop, at whatever loudness the
                // lagging level reads on that frame (it read 0.09 on the first
                // kick of a synthetic drop).
                let drop = kick.is_some() && self.quiet > DROP_QUIET;
                let long = kick.is_some() && self.held > AUTO_HOLD;
                if self.morph >= 1.0 && self.held > MIN_HOLD && (drop || long) {
                    self.start((self.form + 1) % FORMS.len());
                }
            }
        }
        let was = self.morph;
        if self.morph < 1.0 {
            self.morph = (self.morph + dt / MORPH_SECONDS).min(1.0);
        }
        // The knot turns a little faster through a change, so the bloom into
        // the next form reads as one gesture rather than a swap.
        let bell = if was < 1.0 { self.pose().flare } else { 0.0 };
        let drift = frame.setting(id, control::DRIFT);
        self.spin += f64::from(
            dt * (drift * (0.16 + 0.30 * self.energy * response.min(1.0))
                + self.lurch
                + 1.8 * bell),
        );
        self.lurch *= (-dt * 3.0).exp();
    }
}

#[must_use]
pub fn descriptor() -> SceneDescriptor {
    SceneDescriptor {
        id: SceneId::LumenKnot,
        // State version 2: the knot replaced the six fixed ribbons.
        state_version: 2,
        make_state: |seed| Box::new(LumenKnotState::new(seed)),
    }
}

/// The smallest distance between two strand points at least `apart` (a share
/// of the strand) from each other: twice the tube or more means the tube never
/// passes through itself.
#[must_use]
pub fn clearance(
    pose: &Pose,
    bands: &[f32; BANDS],
    response: f32,
    samples: usize,
    apart: f32,
) -> f32 {
    let points: Vec<[f32; 3]> = (0..samples)
        .map(|i| breathe(point(pose, i as f32 / samples as f32), bands, response))
        .collect();
    let skip = (apart * samples as f32).ceil() as usize;
    let mut best = f32::INFINITY;
    for i in 0..samples {
        for j in i + skip..samples {
            if samples - (j - i) < skip {
                continue;
            }
            let d = (0..3)
                .map(|k| (points[i][k] - points[j][k]).powi(2))
                .sum::<f32>()
                .sqrt();
            best = best.min(d);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneSettings;

    fn radius(p: [f32; 3]) -> f32 {
        p[0].hypot(p[1])
    }

    #[test]
    fn forms_are_the_torus_knots_they_claim() {
        // Pinned landmarks of the six-petal rosette, T(5,6): the strand starts
        // at the outer tip of a petal, on the ring's plane.
        let pose = settled(0);
        let p = point(&pose, 0.0);
        assert!((p[0] - 0.88).abs() < 1e-6 && p[1].abs() < 1e-6 && p[2].abs() < 1e-6);
        // A twelfth of the way round the swing (q t = pi/2) it is on the ring,
        // as deep as it gets: 0.5 * 0.30.
        let p = point(&pose, 1.0 / 24.0);
        assert!((radius(p) - 0.58).abs() < 1e-5, "{p:?}");
        assert!((p[2] - 0.15).abs() < 1e-6, "{p:?}");
        // Half a swing on, it is at the inner turn.
        let p = point(&pose, 1.0 / 12.0);
        assert!((radius(p) - 0.28).abs() < 1e-5, "{p:?}");
        // Every form closes on itself and is p-fold rotationally symmetric in
        // the sense that matters: q petals.
        for (i, form) in FORMS.iter().enumerate() {
            let pose = settled(i);
            let (a, b) = (point(&pose, 0.0), point(&pose, 1.0));
            assert!(
                (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5),
                "{} is not closed",
                form.name
            );
            let r = |k: i32| radius(point(&pose, k.rem_euclid(4096) as f32 / 4096.0));
            let tips = (0..4096_i32)
                .filter(|&j| r(j) > r(j - 1) && r(j) >= r(j + 1))
                .count();
            assert_eq!(tips as u32, form.petals, "{}", form.name);
        }
    }

    #[test]
    fn every_form_keeps_its_opening_its_frame_and_its_own_tube_clear() {
        let peaks = [[1.0; BANDS], [1.0, 0.0, 1.0, 0.0, 1.0, 0.0], [0.0; BANDS]];
        for (i, form) in FORMS.iter().enumerate() {
            for swing in [1.0, 0.6, 0.25, PINCH_FLOOR] {
                let pose = Pose {
                    swing,
                    ..settled(i)
                };
                for j in 0..SEGMENTS {
                    let p = point(&pose, j as f32 / SEGMENTS as f32);
                    let norm = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    assert!(
                        radius(p) - pose.tube > 0.2,
                        "{}: opening closed at {p:?}",
                        form.name
                    );
                    assert!(
                        norm + pose.tube <= EXTENT + 1e-6,
                        "{}: escaped at {p:?}",
                        form.name
                    );
                    for bands in &peaks {
                        let q = breathe(p, bands, 2.0);
                        assert!(radius(q) >= radius(p) && radius(q) <= radius(p) * 1.151);
                        assert_eq!(q[2], p[2], "breathing must not touch depth");
                    }
                }
            }
            // Settled, at the strongest breathing, the tube never meets
            // itself: everything a twelfth of the strand away is more than a
            // tube's width off, with room for the woven gap.
            for bands in &peaks {
                let gap = clearance(&settled(i), bands, 2.0, 720, 1.0 / 12.0);
                assert!(
                    gap > 2.6 * form.tube,
                    "{}: tube meets itself ({gap} vs {})",
                    form.name,
                    form.tube
                );
            }
        }
    }

    #[test]
    fn crossings_are_deep_enough_to_weave() {
        // Two passes cross where their radii agree; for a torus knot that
        // means opposite swing phases, so they sit at +z and -z. Find every
        // crossing in the projection and check the passes are more than a
        // tube apart in depth.
        for (i, form) in FORMS.iter().enumerate() {
            let pose = settled(i);
            let n = 1440;
            let pts: Vec<[f32; 3]> = (0..n).map(|j| point(&pose, j as f32 / n as f32)).collect();
            let mut crossings = 0;
            for a in 0..n {
                for b in a + n / 24..n {
                    if n - (b - a) < n / 24 {
                        continue;
                    }
                    let d = (pts[a][0] - pts[b][0]).hypot(pts[a][1] - pts[b][1]);
                    if d < 0.01 {
                        crossings += 1;
                        assert!(
                            (pts[a][2] - pts[b][2]).abs() > 2.0 * form.tube,
                            "{}: shallow crossing at {a}/{b}",
                            form.name
                        );
                    }
                }
            }
            assert!(crossings > 0, "{}: no crossings found", form.name);
        }
    }

    fn run(state: &mut LumenKnotState, frame: &mut SceneFrame<'_>, seconds: f32) {
        let steps = (seconds * 60.0).round() as usize;
        for _ in 0..steps {
            frame.time_seconds += 1.0 / 60.0;
            state.update(frame);
        }
    }

    #[test]
    fn a_named_form_starts_tied_and_a_change_pinches_through_a_ring() {
        let mut settings = SceneSettings::default();
        assert!(settings.set(SceneId::LumenKnot, control::FORM, 3.0));
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        let mut s = LumenKnotState::new(7);
        s.update(&frame);
        assert_eq!((s.form, s.morph), (2, 1.0), "a fresh state must open tied");
        assert_eq!(s.pose(), settled(2));
        let mut changed = settings;
        assert!(changed.set(SceneId::LumenKnot, control::FORM, 5.0));
        let mut frame = SceneFrame::idle(&changed);
        frame.delta_seconds = 1.0 / 60.0;
        let mut tightest = 1.0_f32;
        let mut shown = Vec::new();
        for _ in 0..(MORPH_SECONDS * 60.0) as usize + 4 {
            frame.time_seconds += 1.0 / 60.0;
            s.update(&frame);
            let pose = s.pose();
            tightest = tightest.min(pose.swing);
            if shown.last() != Some(&pose.form) {
                shown.push(pose.form);
            }
        }
        assert_eq!(shown, vec![2, 4], "one swap, from the old form to the new");
        assert!(
            tightest <= PINCH_FLOOR + 0.02,
            "never pulled tight: {tightest}"
        );
        assert_eq!(s.morph, 1.0);
        assert_eq!(s.pose(), settled(4));
    }

    #[test]
    fn the_song_moves_the_form_on_at_a_drop_not_before() {
        let settings = SceneSettings::default();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        let mut s = LumenKnotState::new(1);
        // Quiet for eight seconds: no change.
        run(&mut s, &mut frame, 8.0);
        assert_eq!((s.form, s.morph), (0, 1.0));
        // Then a loud kick: low bands leap above their trails.
        let loud = [1.0_f32; 120];
        let mut trail = [0.0_f32; 120];
        trail[30..].fill(1.0);
        frame.audio.bands = &loud;
        frame.audio.trails = &trail;
        frame.audio.rms = 0.9;
        frame.time_seconds += 1.0 / 60.0;
        s.update(&frame);
        assert_eq!(s.form, 1, "a drop after quiet must move on");
        assert!(s.morph < 1.0);
        // Response 0 disables the musical coupling, cycle included.
        let mut off = SceneSettings::default();
        assert!(off.set(SceneId::LumenKnot, control::RESPONSE, 0.0));
        let mut frame = SceneFrame::idle(&off);
        frame.delta_seconds = 1.0 / 60.0;
        let mut s = LumenKnotState::new(1);
        run(&mut s, &mut frame, 8.0);
        frame.audio.bands = &loud;
        frame.audio.trails = &trail;
        frame.audio.rms = 0.9;
        frame.time_seconds += 1.0 / 60.0;
        s.update(&frame);
        assert_eq!(s.form, 0);
    }

    #[test]
    fn a_loud_loop_with_dips_between_kicks_is_not_a_drop() {
        // Kicks every half second whose loudness falls to nothing between
        // them: on the instantaneous level the dips summed to "quiet" and the
        // form moved on every seven seconds or so.
        let settings = SceneSettings::default();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        let mut s = LumenKnotState::new(1);
        let hit = [1.0_f32; 120];
        let mut trail = [0.0_f32; 120];
        trail[30..].fill(1.0);
        let rest = [0.0_f32; 120];
        for step in 0..(18 * 60) {
            frame.time_seconds += 1.0 / 60.0;
            let beat = step % 30;
            if beat < 2 {
                frame.audio.bands = &hit;
                frame.audio.trails = &trail;
                frame.audio.rms = 0.6;
            } else {
                frame.audio.bands = &rest;
                frame.audio.trails = &rest;
                frame.audio.rms = if beat < 8 { 0.3 } else { 0.02 };
            }
            s.update(&frame);
        }
        assert!(
            s.kick.count() > 20,
            "the loop must be kicking: {}",
            s.kick.count()
        );
        assert_eq!((s.form, s.morph), (0, 1.0), "a steady loop is not a drop");
    }

    #[test]
    fn comets_travel_fade_and_keep_idle_light_moving() {
        let settings = SceneSettings::default();
        let mut frame = SceneFrame::idle(&settings);
        frame.delta_seconds = 1.0 / 60.0;
        let mut s = LumenKnotState::new(3);
        run(&mut s, &mut frame, AMBIENT_GAP + 0.1);
        assert_eq!(s.fired, 1, "silence still sends a soft comet");
        let head = s.comets[0].head;
        run(&mut s, &mut frame, 1.0);
        let moved = (s.comets[0].head - head).rem_euclid(1.0);
        let expected = COMET_SPEED * 0.70;
        assert!((moved - expected).abs() < 0.002, "{moved} vs {expected}");
        run(&mut s, &mut frame, COMET_SECONDS);
        assert!(s.comets.iter().all(|c| c.age < COMET_SECONDS));
        assert!(s.comets.len() <= COMETS);
    }

    #[test]
    fn followers_are_selective_deterministic_and_release() {
        let settings = SceneSettings::default();
        let mut frame = SceneFrame::idle(&settings);
        let mut audio = [0.0; 120];
        audio[40..60].fill(1.0);
        frame.audio.bands = &audio;
        frame.delta_seconds = 1.0 / 60.0;
        let mut a = LumenKnotState::new(42);
        let mut b = a.clone();
        for i in 0..60 {
            frame.time_seconds = i as f64 / 60.0;
            a.update(&frame);
            b.update(&frame);
        }
        assert_eq!(a, b);
        assert!(a.bands[2] > 0.99);
        assert_eq!(a.bands[0], 0.0);
        assert_eq!(a.bands[5], 0.0);
        frame.audio.bands = &[];
        for _ in 0..180 {
            a.update(&frame);
        }
        assert!(a.energy < 0.0001);
    }

    #[test]
    fn bad_audio_and_zero_dt_do_not_poison_state() {
        let settings = SceneSettings::default();
        let mut f = SceneFrame::idle(&settings);
        f.audio.bands = &[f32::NAN, f32::INFINITY, -1.0];
        f.delta_seconds = 0.016;
        let mut s = LumenKnotState::new(0);
        s.update(&f);
        assert_eq!(s.energy, 0.0);
        assert!(s.spin.is_finite());
        f.audio.bands = &[1.0; 120];
        f.delta_seconds = 0.0;
        let before = s.clone();
        s.update(&f);
        assert_eq!(s, before);
    }

    #[test]
    fn unreadable_form_values_fall_back_to_the_song() {
        assert_eq!(form_from_setting(0.0), None);
        assert_eq!(form_from_setting(f32::NAN), None);
        assert_eq!(form_from_setting(1.0), Some(0));
        assert_eq!(form_from_setting(6.0), Some(5));
        assert_eq!(form_from_setting(7.0), None);
    }
}
