//! How a section of the scene plan arrives, and how a video begins and ends.
//!
//! Post-legacy (2026-10-12, MV3). The C's plan changes scene on a frame
//! boundary and that is all it can do; so could this one, until a video
//! directed from the command line made every change a hard cut and the first
//! and last frames a hard start and stop.
//!
//! A [`Transition`] belongs to the section it brings *in*. The first section's
//! is the video's intro, played over black, and the plan carries one more for
//! the end of the track. Everything here is arithmetic over the playhead:
//! [`resolve`] says what to composite for a given moment and the drawing half
//! does it, so a still, a clip and a full export agree by construction.
//!
//! ## The outgoing picture is held, not replayed
//!
//! A transition composites the last frame the outgoing section drew over the
//! incoming section, which is live. Running both scenes at once would need two
//! settings tables, two route evaluations and scene state advanced past its
//! own cut; a held frame that zooms, slides, burns or shatters away inside
//! half a second is the same picture to an eye and a tenth of the machinery.

use std::fmt;

/// Longest transition a plan may ask for, in seconds.
pub const MAX_SECONDS: f32 = 8.0;
/// Shortest one that is still a transition rather than a cut.
pub const MIN_SECONDS: f32 = 0.05;
/// What a kind runs for when the author names it and gives no length.
pub const DEFAULT_SECONDS: f32 = 0.6;

/// The look of one transition. The numeric value is what the shader switches
/// on (`resources/shaders/glsl330/transition.fs`), so the order is a contract
/// between the two files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TransitionKind {
    /// No transition: the C's frame-boundary change.
    #[default]
    Cut = 0,
    /// A dissolve.
    Fade = 1,
    /// The outgoing picture blows out to white, which decays onto the new one.
    Flash = 2,
    /// A soft diagonal edge sweeps across, lit where it passes.
    Wipe = 3,
    /// A circle opens from the centre.
    Iris = 4,
    /// The outgoing picture rushes toward the viewer and thins out.
    Zoom = 5,
    /// Horizontal bands slide away, alternately left and right.
    Slices = 6,
    /// Blocks tear sideways, split into colour fringes and drop out.
    Glitch = 7,
    /// The picture burns away along a ragged, glowing edge.
    Burn = 8,
    /// The picture coarsens into growing blocks and fades.
    Pixel = 9,
    /// Down to black and back up, half on each side of the change.
    Dip = 10,
    /// Vertical blinds turn open.
    Blinds = 11,
    /// A ring of light ripples out from the centre and takes the picture with it.
    Ripple = 12,
}

impl TransitionKind {
    /// Every kind, in shader order.
    pub const ALL: [Self; 13] = [
        Self::Cut,
        Self::Fade,
        Self::Flash,
        Self::Wipe,
        Self::Iris,
        Self::Zoom,
        Self::Slices,
        Self::Glitch,
        Self::Burn,
        Self::Pixel,
        Self::Dip,
        Self::Blinds,
        Self::Ripple,
    ];

    /// The persisted token, and the one a treatment names.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cut => "cut",
            Self::Fade => "fade",
            Self::Flash => "flash",
            Self::Wipe => "wipe",
            Self::Iris => "iris",
            Self::Zoom => "zoom",
            Self::Slices => "slices",
            Self::Glitch => "glitch",
            Self::Burn => "burn",
            Self::Pixel => "pixel",
            Self::Dip => "dip",
            Self::Blinds => "blinds",
            Self::Ripple => "ripple",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// One line on how it looks, for `--vocabulary`.
    #[must_use]
    pub fn look(self) -> &'static str {
        match self {
            Self::Cut => "No transition; the picture changes on one frame.",
            Self::Fade => "A dissolve from the old picture to the new.",
            Self::Flash => {
                "The old picture blows out to white and the white decays onto the new \
                 one. Made for a drop; keep it short."
            }
            Self::Wipe => "A soft diagonal edge sweeps across, glowing where it passes.",
            Self::Iris => "A circle opens from the centre onto the new picture.",
            Self::Zoom => "The old picture rushes at the viewer and thins out.",
            Self::Slices => "Horizontal bands slide away, alternately left and right.",
            Self::Glitch => {
                "Blocks tear sideways with colour fringes and drop out a few at a time."
            }
            Self::Burn => "The old picture burns away along a ragged, glowing edge.",
            Self::Pixel => "The old picture coarsens into growing blocks and fades.",
            Self::Dip => {
                "Down to black and back up, half before the change and half after. \
                 The one kind that starts before its section does."
            }
            Self::Blinds => "Vertical blinds turn open onto the new picture.",
            Self::Ripple => {
                "A ring of light spreads from the centre and carries the old picture off."
            }
        }
    }
}

impl fmt::Display for TransitionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One authored transition.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transition {
    pub kind: TransitionKind,
    /// How long it runs. Ignored for [`TransitionKind::Cut`].
    pub seconds: f32,
}

impl Transition {
    /// The C's behaviour, and the default nothing serializes.
    pub const CUT: Self = Self {
        kind: TransitionKind::Cut,
        seconds: 0.0,
    };

    #[must_use]
    pub fn new(kind: TransitionKind, seconds: f32) -> Self {
        if kind == TransitionKind::Cut {
            Self::CUT
        } else {
            Self { kind, seconds }
        }
    }

    #[must_use]
    pub fn is_cut(self) -> bool {
        self.kind == TransitionKind::Cut
    }

    /// Whether this is a value a plan may hold: a cut, or a kind with a finite
    /// length inside the bounds.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.is_cut()
            || (self.seconds.is_finite() && (MIN_SECONDS..=MAX_SECONDS).contains(&self.seconds))
    }
}

/// What the held picture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The last frame the outgoing section drew.
    Previous,
    /// Black: the video's own beginning or end, or a dip.
    Black,
}

/// What to composite over the live scene at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Active {
    pub kind: TransitionKind,
    pub source: Source,
    /// 0 when the held picture covers the frame, 1 when it is gone. An outro
    /// runs this backwards, so every kind leaves the way it would arrive.
    pub progress: f32,
    /// Stable per transition, so a kind with random structure draws the same
    /// structure on every frame of one transition and a different one at the
    /// next.
    pub seed: u32,
    /// The moment the held frame must have been drawn before, for a
    /// [`Source::Previous`]: the cut itself.
    pub cut_seconds: f64,
}

/// One section boundary, as [`resolve`] needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Section {
    pub id: u64,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub transition: Transition,
}

/// A transition's length once it has to fit the section it brings in.
///
/// A two-second dissolve into a one-second section would still be running when
/// the next change arrives, holding a picture from two sections ago. It gets
/// the section and no more.
#[must_use]
pub fn fitted_seconds(transition: Transition, section_seconds: f64) -> f64 {
    if transition.is_cut() {
        return 0.0;
    }
    f64::from(transition.seconds).min(section_seconds.max(0.0))
}

/// The transition in force at `time_seconds`, if any.
///
/// `sections` is the enabled plan in order. Precedence where two windows meet:
/// the outro wins over everything, because the end of the track is the one
/// boundary that cannot be late, and a dip's first half belongs to the section
/// *before* its own.
#[must_use]
pub fn resolve(
    sections: &[Section],
    outro: Transition,
    duration_seconds: f64,
    time_seconds: f64,
) -> Option<Active> {
    if !time_seconds.is_finite() || !duration_seconds.is_finite() {
        return None;
    }
    if !outro.is_cut() && duration_seconds > 0.0 {
        let length = f64::from(outro.seconds).min(duration_seconds);
        let begin = duration_seconds - length;
        if length > 0.0 && time_seconds >= begin {
            let elapsed = ((time_seconds - begin) / length).clamp(0.0, 1.0) as f32;
            return Some(Active {
                kind: if outro.kind == TransitionKind::Dip {
                    TransitionKind::Fade
                } else {
                    outro.kind
                },
                source: Source::Black,
                progress: 1.0 - elapsed,
                seed: 0x6f75_7472,
                cut_seconds: duration_seconds,
            });
        }
    }

    for (index, section) in sections.iter().enumerate() {
        let transition = section.transition;
        if transition.is_cut() {
            continue;
        }
        let cut = section.start_seconds;
        let room = section.end_seconds - section.start_seconds;
        let seed = (section.id as u32) ^ ((section.id >> 32) as u32) ^ 0x9e37_79b9;

        if transition.kind == TransitionKind::Dip {
            // Half before the change, half after; the first section has no
            // "before", so its dip is a plain rise out of black.
            let half = fitted_seconds(transition, room * 2.0) * 0.5;
            if half <= 0.0 {
                continue;
            }
            let before = if index == 0 {
                0.0
            } else {
                half.min(cut - sections[index - 1].start_seconds)
            };
            if time_seconds >= cut - before && time_seconds < cut + half {
                let progress = if time_seconds < cut {
                    // Falling: the black is arriving, so this runs backwards.
                    ((cut - time_seconds) / before).clamp(0.0, 1.0) as f32
                } else {
                    ((time_seconds - cut) / half).clamp(0.0, 1.0) as f32
                };
                return Some(Active {
                    kind: TransitionKind::Fade,
                    source: Source::Black,
                    progress,
                    seed,
                    cut_seconds: cut,
                });
            }
            continue;
        }

        let length = fitted_seconds(transition, room);
        if length > 0.0 && time_seconds >= cut && time_seconds < cut + length {
            return Some(Active {
                kind: transition.kind,
                source: if index == 0 {
                    Source::Black
                } else {
                    Source::Previous
                },
                progress: ((time_seconds - cut) / length).clamp(0.0, 1.0) as f32,
                seed,
                cut_seconds: cut,
            });
        }
    }
    None
}

/// Whether any section of the plan needs the outgoing picture held, which is
/// what tells the drawing half to keep one.
#[must_use]
pub fn needs_previous(sections: &[Section]) -> bool {
    sections.iter().enumerate().any(|(index, section)| {
        index > 0 && !section.transition.is_cut() && section.transition.kind != TransitionKind::Dip
    })
}

/// How far back a replay must start drawing for the frame at `time_seconds`
/// to be the frame a full render would have produced, in seconds.
///
/// A transition holds the frame drawn just before its cut. A still or a clip
/// that starts inside one therefore has to have drawn that frame, and
/// `scene_warmup` more before it for a scene that keeps light of its own.
#[must_use]
pub fn warmup_seconds(active: Option<Active>, time_seconds: f64, scene_warmup: f64) -> f64 {
    match active {
        Some(active) if active.source == Source::Previous => {
            (time_seconds - active.cut_seconds).max(0.0) + scene_warmup + 0.25
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(id: u64, start: f64, end: f64, kind: TransitionKind, seconds: f32) -> Section {
        Section {
            id,
            start_seconds: start,
            end_seconds: end,
            transition: Transition::new(kind, seconds),
        }
    }

    fn plan() -> Vec<Section> {
        vec![
            section(1, 0.0, 10.0, TransitionKind::Iris, 2.0),
            section(2, 10.0, 20.0, TransitionKind::Burn, 1.0),
            section(3, 20.0, 20.5, TransitionKind::Fade, 4.0),
            section(4, 20.5, 40.0, TransitionKind::Cut, 0.0),
            section(5, 40.0, 60.0, TransitionKind::Dip, 2.0),
        ]
    }

    #[test]
    fn every_kind_has_a_unique_name_that_round_trips_and_a_shader_index() {
        for (index, kind) in TransitionKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, index, "{kind} is out of shader order");
            assert_eq!(TransitionKind::from_name(kind.name()), Some(kind));
            assert!(!kind.look().is_empty());
        }
        assert_eq!(TransitionKind::from_name("crossfade"), None);
        assert_eq!(Transition::new(TransitionKind::Cut, 3.0), Transition::CUT);
    }

    #[test]
    fn the_first_section_arrives_out_of_black_and_later_ones_out_of_the_last_frame() {
        let plan = plan();
        let intro = resolve(&plan, Transition::CUT, 60.0, 0.5).unwrap();
        assert_eq!(
            (intro.kind, intro.source),
            (TransitionKind::Iris, Source::Black)
        );
        assert!((intro.progress - 0.25).abs() < 1.0e-6);

        let burn = resolve(&plan, Transition::CUT, 60.0, 10.25).unwrap();
        assert_eq!(
            (burn.kind, burn.source),
            (TransitionKind::Burn, Source::Previous)
        );
        assert!((burn.progress - 0.25).abs() < 1.0e-6);
        assert_eq!(burn.cut_seconds, 10.0);
        assert_ne!(burn.seed, intro.seed);

        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 5.0), None);
        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 9.999), None);
        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 30.0), None);
    }

    #[test]
    fn a_transition_never_outlasts_the_section_it_brings_in() {
        let plan = plan();
        // Four seconds were asked for and the section is half a second long.
        let inside = resolve(&plan, Transition::CUT, 60.0, 20.25).unwrap();
        assert!((inside.progress - 0.5).abs() < 1.0e-6);
        // The moment the next section starts, the fade is over.
        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 20.5), None);
    }

    #[test]
    fn a_dip_falls_before_the_change_and_rises_after_it() {
        let plan = plan();
        let falling = resolve(&plan, Transition::CUT, 60.0, 39.5).unwrap();
        assert_eq!(falling.source, Source::Black);
        assert!((falling.progress - 0.5).abs() < 1.0e-6);
        let black = resolve(&plan, Transition::CUT, 60.0, 40.0).unwrap();
        assert_eq!(black.progress, 0.0, "fully black on the change itself");
        let rising = resolve(&plan, Transition::CUT, 60.0, 40.75).unwrap();
        assert!((rising.progress - 0.75).abs() < 1.0e-6);
        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 38.9), None);
        assert_eq!(resolve(&plan, Transition::CUT, 60.0, 41.0), None);
        // A dip never needs the outgoing frame held.
        assert!(!needs_previous(&plan[4..]));
        assert!(needs_previous(&plan));
    }

    #[test]
    fn the_outro_runs_its_kind_backwards_into_black_and_wins_at_the_end() {
        let plan = plan();
        let outro = Transition::new(TransitionKind::Iris, 4.0);
        assert_eq!(resolve(&plan, outro, 60.0, 55.9), None);
        let closing = resolve(&plan, outro, 60.0, 57.0).unwrap();
        assert_eq!(
            (closing.kind, closing.source),
            (TransitionKind::Iris, Source::Black)
        );
        assert!((closing.progress - 0.75).abs() < 1.0e-6);
        let last = resolve(&plan, outro, 60.0, 60.0).unwrap();
        assert_eq!(last.progress, 0.0, "black on the final frame");
        // With no plan at all an outro still applies: a track can end softly
        // without being cut into sections.
        assert!(resolve(&[], outro, 60.0, 59.0).is_some());
    }

    #[test]
    fn validity_is_a_cut_or_a_bounded_length() {
        assert!(Transition::CUT.is_valid());
        assert!(Transition::new(TransitionKind::Fade, 0.6).is_valid());
        assert!(!Transition::new(TransitionKind::Fade, 0.0).is_valid());
        assert!(!Transition::new(TransitionKind::Fade, 9.0).is_valid());
        assert!(!Transition::new(TransitionKind::Fade, f32::NAN).is_valid());
    }

    #[test]
    fn a_replay_inside_a_transition_starts_before_the_cut() {
        let plan = plan();
        let active = resolve(&plan, Transition::CUT, 60.0, 10.5);
        let warmup = warmup_seconds(active, 10.5, 3.0);
        assert!(warmup >= 3.5, "{warmup}");
        assert_eq!(warmup_seconds(None, 10.5, 3.0), 0.0);
        let intro = resolve(&plan, Transition::CUT, 60.0, 0.5);
        assert_eq!(
            warmup_seconds(intro, 0.5, 3.0),
            0.0,
            "black needs no run-up"
        );
    }
}
