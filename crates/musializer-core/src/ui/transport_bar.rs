//! Where the transport row's control clusters go.
//!
//! **Not a port.** The frozen C's toolbar is six text buttons and a timecode, laid
//! out by [`timeline_layout::TimelineBand`] alone. This adds a right-hand cluster —
//! mute, volume, the diagnostic readout toggle, fullscreen — and a fine-seek group
//! beside the transport button, neither of which the oracle has. It is written as a
//! module here rather than as arithmetic beside the drawing for the reason the rest
//! of `core::ui` exists: a row that overflows at 960 px with the inspector open is
//! a real reachable state, and the only way to check it at every width is to be
//! able to compute it without a window.
//!
//! The band still owns the middle. This module's whole job is to decide what is
//! *left* for it, which keeps one owner for the question the C repository already
//! paid for twice: whether the timecode can sit inline, or whether it belongs to
//! the timeline panel instead (`timeline_layout.h:12-21`).
//!
//! # The shedding order
//!
//! Below a certain width not everything fits, and the alternative to shedding is
//! the defect the oracle's own header names — controls squeezed until their labels
//! are unreadable. Clusters are dropped whole rather than individually, in this
//! order, and the reason for the order is what each control costs the user:
//!
//! 1. **Fine seek** goes first. Every one of its actions has a keyboard binding
//!    (Home, arrows, Ctrl/Shift), and the hint line beside the timeline still
//!    names them, so dropping it removes no capability.
//! 2. **The volume slider** goes next, leaving the mute button. Muting is the
//!    common case and the slider is the wide part.
//! 3. **Mute and the readout toggle** go together, both being keyboard-bound.
//!
//! Fullscreen is never shed. It is the control that gets a cramped window *out* of
//! being cramped, so it is the last thing a narrow row should lose.

use super::workspace_layout::UiRect;

/// A square control box, sized from the row height.
///
/// Icons are square, unlike text labels, which is the entire reason this row can
/// carry eleven controls where the oracle's carries six.
pub const CONTROL_SIZE: f32 = 34.0;

/// The volume slider's width when it is drawn at full size.
pub const VOLUME_SLIDER_WIDTH: f32 = 96.0;

/// Below this, the volume slider is dropped and mute stands alone.
pub const VOLUME_SLIDER_MIN_WIDTH: f32 = 64.0;

/// Gap between controls inside one cluster.
pub const CLUSTER_GAP: f32 = 4.0;

/// Gap between clusters, which is wider so the groups read as groups.
pub const GROUP_GAP: f32 = 16.0;

/// What the right-hand cluster placed, and what it took from the bar.
///
/// Every rectangle is `None` when that control was shed, rather than zero-sized:
/// a zero-sized rect still round-trips through a draw call and would claim presses
/// at the window origin, which is the class of bug
/// [`super::workspace_layout`] exists to prevent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UtilityCluster {
    pub readout: Option<UiRect>,
    pub mute: Option<UiRect>,
    pub volume: Option<UiRect>,
    /// Never `None`: fullscreen is the one control this row will not shed.
    pub fullscreen: UiRect,
    /// The x the rest of the row must stop before.
    pub left_edge: f32,
}

/// The fine-seek group: jump to start, nudge back, nudge forward.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeekCluster {
    pub start: UiRect,
    pub back: UiRect,
    pub forward: UiRect,
    /// Total width including the trailing group gap.
    pub width: f32,
}

/// One rung of the shedding ladder: which controls the cluster carries, and how
/// wide the volume slider is if it carries one.
///
/// Written as whole configurations rather than as a sequence of "place it if it
/// fits" decisions, because greedy placement is **not monotone in width** and the
/// difference is visible: with the slider falling back from 96 px to 64 px, there
/// is a window width at which dropping the slider entirely frees enough room for
/// the mute button to reappear. Shrinking the window then made a control vanish
/// and come back. The sweep in this module's tests caught that on the first run,
/// and picking the richest configuration that fits makes the defect unstateable —
/// the widths below strictly decrease, so the choice is monotone by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rung {
    Full,
    NarrowSlider,
    NoSlider,
    FullscreenOnly,
}

impl Rung {
    /// Richest first. A configuration is chosen by walking this in order and
    /// taking the first that fits.
    const LADDER: [Rung; 4] = [
        Rung::Full,
        Rung::NarrowSlider,
        Rung::NoSlider,
        Rung::FullscreenOnly,
    ];

    /// The slider width this rung asks for, or `None` for no slider.
    fn slider(self) -> Option<f32> {
        match self {
            Rung::Full => Some(VOLUME_SLIDER_WIDTH),
            Rung::NarrowSlider => Some(VOLUME_SLIDER_MIN_WIDTH),
            Rung::NoSlider | Rung::FullscreenOnly => None,
        }
    }

    /// Whether mute and the readout toggle are carried. They travel together: a
    /// mute button with no slider and no neighbour reads as a stray, and the pair
    /// is what makes the cluster legible as a cluster.
    fn has_buttons(self) -> bool {
        !matches!(self, Rung::FullscreenOnly)
    }

    /// Total width including the gaps between controls, but not the group gap.
    fn width(self) -> f32 {
        let mut width = CONTROL_SIZE;
        if self.has_buttons() {
            width += (CONTROL_SIZE + CLUSTER_GAP) * 2.0;
        }
        if let Some(slider) = self.slider() {
            width += slider + CLUSTER_GAP;
        }
        width
    }
}

/// Lays the utility cluster out against the right edge of `bar`.
///
/// Returns `None` when the bar cannot even seat the fullscreen button, which is
/// the caller's signal to draw no utilities at all rather than to draw a
/// degenerate one.
///
/// `want_volume` is false when there is no stream to set a volume on; the cluster
/// then starts at [`Rung::NoSlider`] rather than drawing a slider that writes
/// nowhere.
#[must_use]
pub fn utilities(bar: UiRect, row_y: f32, want_volume: bool) -> Option<UtilityCluster> {
    if !bar.is_finite() || bar.width <= 0.0 {
        return None;
    }
    // What the middle group must be left: one control box plus a group gap, which
    // is the lone transport button the band falls back to when nothing else fits.
    const MIDDLE_RESERVE: f32 = CONTROL_SIZE + GROUP_GAP;

    let available = bar.width - CLUSTER_GAP;
    let rung = Rung::LADDER.into_iter().find(|rung| {
        if !want_volume && rung.slider().is_some() {
            return false;
        }
        // The last rung is allowed to consume the reserve: a bar that can seat
        // only the fullscreen button has no middle group to protect.
        let reserve = if *rung == Rung::FullscreenOnly {
            0.0
        } else {
            MIDDLE_RESERVE
        };
        rung.width() + reserve <= available
    })?;

    // Placed right to left, because the right edge is the fixed thing: the middle
    // group is what absorbs a width change.
    let mut right = bar.x + bar.width - CLUSTER_GAP;
    let mut take = |width: f32| {
        right -= width;
        let rect = UiRect::new(right, row_y, width, CONTROL_SIZE);
        right -= CLUSTER_GAP;
        rect
    };

    let fullscreen = take(CONTROL_SIZE);
    let volume = rung.slider().map(&mut take);
    let (mute, readout) = if rung.has_buttons() {
        (Some(take(CONTROL_SIZE)), Some(take(CONTROL_SIZE)))
    } else {
        (None, None)
    };

    Some(UtilityCluster {
        readout,
        mute,
        volume,
        fullscreen,
        // `right` has already had a trailing cluster gap removed by `take`, so
        // this is the last control's left edge minus the two gaps that separate
        // the clusters.
        left_edge: right + CLUSTER_GAP - GROUP_GAP,
    })
}

/// Lays the fine-seek group out at `x`, or reports that it does not fit.
///
/// `available` is what is left of the row after the transport button and the panel
/// buttons have taken theirs — this group is shed first, so it is the one asked to
/// justify itself against what remains.
#[must_use]
pub fn seek_group(x: f32, row_y: f32, available: f32) -> Option<SeekCluster> {
    let width = CONTROL_SIZE * 3.0 + CLUSTER_GAP * 2.0;
    if available < width + GROUP_GAP {
        return None;
    }
    let step = CONTROL_SIZE + CLUSTER_GAP;
    Some(SeekCluster {
        start: UiRect::new(x, row_y, CONTROL_SIZE, CONTROL_SIZE),
        back: UiRect::new(x + step, row_y, CONTROL_SIZE, CONTROL_SIZE),
        forward: UiRect::new(x + step * 2.0, row_y, CONTROL_SIZE, CONTROL_SIZE),
        width: width + GROUP_GAP,
    })
}

/// The seek groove's height: the part of the position bar that is a control.
pub const SCRUB_TRACK_HEIGHT: f32 = 8.0;
/// The analyzer level strip embedded along the bar's bottom edge.
pub const SCRUB_LEVEL_HEIGHT: f32 = 4.0;
/// Between the two, so the level reads as an annotation rather than as a second
/// control.
pub const SCRUB_LEVEL_GAP: f32 = 2.0;
/// Total height of the position bar.
pub const SCRUB_BAR_HEIGHT: f32 = SCRUB_TRACK_HEIGHT + SCRUB_LEVEL_GAP + SCRUB_LEVEL_HEIGHT;
/// Below this the bar is not drawn at all.
///
/// A 20 px seek bar is a decoration that invites a click it cannot serve
/// precisely — one pixel would be several seconds of a track. The number is the
/// meter's old floor, kept: the reason for a floor did not change when the
/// readout became a control.
pub const SCRUB_BAR_MIN_WIDTH: f32 = 60.0;
/// Below this there is no room for the telemetry caption above the bar.
pub const SCRUB_CAPTION_MIN_WIDTH: f32 = 190.0;

/// The transport row's position bar (review 1.9, UX0-A09).
///
/// **Not the oracle's.** What sat here was an RMS level meter — a green fill
/// growing from the left, beside the timecode, exactly where every media player
/// puts progress, and with no widget id at all. At 0.15 s into a track it read
/// about 22% full, so the interface's most conventional-looking control was both
/// unclickable and lying about the position. It is a seek bar now, and the level
/// it used to show is [`ScrubBar::level`]: a 4 px strip under the groove, which
/// is legible as a level and not mistakable for progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrubBar {
    /// The whole box, and the press area. Taller than the groove so the control
    /// is not a 8 px target.
    pub hit: UiRect,
    /// The seek groove: background, progress fill and playhead.
    pub track: UiRect,
    /// The analyzer's level, along the bottom edge.
    pub level: UiRect,
}

/// Lays the position bar into what the row has left between `left` and `right`.
///
/// `None` when that is narrower than [`SCRUB_BAR_MIN_WIDTH`], which is the
/// caller's signal to draw nothing rather than a stub.
#[must_use]
pub fn scrub_bar(left: f32, right: f32, bar: UiRect) -> Option<ScrubBar> {
    if !bar.is_finite() || !left.is_finite() || !right.is_finite() {
        return None;
    }
    let width = right - left;
    if width < SCRUB_BAR_MIN_WIDTH || bar.height < SCRUB_BAR_HEIGHT {
        return None;
    }
    let top = bar.y + (bar.height - SCRUB_BAR_HEIGHT) * 0.5;
    Some(ScrubBar {
        hit: UiRect::new(left, top, width, SCRUB_BAR_HEIGHT),
        track: UiRect::new(left, top, width, SCRUB_TRACK_HEIGHT),
        level: UiRect::new(
            left,
            top + SCRUB_TRACK_HEIGHT + SCRUB_LEVEL_GAP,
            width,
            SCRUB_LEVEL_HEIGHT,
        ),
    })
}

/// How far along the track the playhead is, in `[0, 1]`.
///
/// Zero for a duration that is not known yet, rather than a fill of whatever
/// `time / 0` produces — a bar that reads full on an unmeasured stream is the
/// same lie the level meter was telling.
#[must_use]
pub fn scrub_fraction(time_seconds: f64, duration_seconds: f64) -> f32 {
    if !time_seconds.is_finite() || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0.0;
    }
    (time_seconds / duration_seconds).clamp(0.0, 1.0) as f32
}

/// The position a press at `x` on `track` asks for, clamped into the track.
///
/// Clamped for the reason [`nudged`] is: a drag past either end is a request for
/// that end, and a scrubber that refuses at the boundary reads as stuck.
#[must_use]
pub fn scrub_seconds(x: f32, track: UiRect, duration_seconds: f64) -> f64 {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0.0;
    }
    if !x.is_finite() || track.width <= 0.0 || !track.is_finite() {
        return 0.0;
    }
    let fraction = f64::from(((x - track.x) / track.width).clamp(0.0, 1.0));
    (fraction * duration_seconds).clamp(0.0, duration_seconds)
}

/// How far one seek nudge moves, given the modifier keys held.
///
/// The three sizes are the ones the reference toolbar in the request spells out,
/// and they are here rather than in the shell so the hint line and the action
/// cannot disagree about what a modifier does — a hint that lies is worse than no
/// hint.
#[must_use]
pub fn seek_step_seconds(fine: bool, coarse: bool) -> f64 {
    match (fine, coarse) {
        // Fine wins when both are held. Holding two modifiers is ambiguous, and
        // the smaller step is the recoverable mistake: overshooting by 10 s when
        // 0.1 s was meant costs the user a scrub back.
        (true, _) => 0.1,
        (false, true) => 10.0,
        (false, false) => 1.0,
    }
}

/// The seek target after a nudge, clamped into the track.
///
/// Clamped rather than wrapped or refused: a nudge past the end is a request to go
/// to the end, and a transport control that silently does nothing at a boundary
/// reads as broken.
#[must_use]
pub fn nudged(time_seconds: f64, step: f64, duration_seconds: f64) -> f64 {
    if !time_seconds.is_finite() || !step.is_finite() {
        return 0.0;
    }
    // A duration of zero or one that is not finite means the track's length is not
    // known yet, and clamping to it would pin the playhead at zero.
    let ceiling = if duration_seconds.is_finite() && duration_seconds > 0.0 {
        duration_seconds
    } else {
        f64::MAX
    };
    (time_seconds + step).clamp(0.0, ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(width: f32) -> UiRect {
        UiRect::new(0.0, 0.0, width, 50.0)
    }

    #[test]
    fn the_utility_cluster_never_leaves_the_bar() {
        // A sweep, because the interesting failures are exactly at the widths where
        // one more control stops fitting.
        for width in 40..=1920 {
            let bar = bar(width as f32);
            let Some(cluster) = utilities(bar, 8.0, true) else {
                continue;
            };
            for (name, rect) in [
                ("fullscreen", Some(cluster.fullscreen)),
                ("mute", cluster.mute),
                ("volume", cluster.volume),
                ("readout", cluster.readout),
            ] {
                let Some(rect) = rect else { continue };
                assert!(
                    rect.x >= bar.x - 0.01,
                    "{width}px: {name} starts at {} left of the bar",
                    rect.x
                );
                assert!(
                    rect.x + rect.width <= bar.x + bar.width + 0.01,
                    "{width}px: {name} ends at {} past the bar",
                    rect.x + rect.width
                );
            }
        }
    }

    #[test]
    fn the_utility_cluster_controls_never_overlap_each_other() {
        for width in 40..=1920 {
            let Some(cluster) = utilities(bar(width as f32), 8.0, true) else {
                continue;
            };
            let placed: Vec<UiRect> = [cluster.readout, cluster.mute, cluster.volume]
                .into_iter()
                .flatten()
                .chain(std::iter::once(cluster.fullscreen))
                .collect();
            for (i, a) in placed.iter().enumerate() {
                for b in &placed[i + 1..] {
                    assert!(!a.overlaps(*b), "{width}px: two controls overlap");
                }
            }
        }
    }

    /// How much the cluster is carrying, as a number that must never rise as the
    /// window narrows.
    fn richness(cluster: &UtilityCluster) -> u32 {
        let mut score = 0;
        if cluster.mute.is_some() {
            score += 1;
        }
        if cluster.readout.is_some() {
            score += 1;
        }
        score += match cluster.volume.map(|v| v.width) {
            Some(width) if width >= VOLUME_SLIDER_WIDTH => 2,
            Some(_) => 1,
            None => 0,
        };
        score
    }

    #[test]
    fn shedding_is_monotone_in_width_so_no_control_vanishes_and_returns() {
        // The defect this exists for, found by this sweep on its first run: with a
        // greedy placer, dropping the volume slider freed enough room for the mute
        // button to reappear, so *narrowing* the window made a control come back.
        // A user resizing across that width saw mute flicker.
        //
        // Stated as monotonicity rather than as "control X is shed before Y",
        // because monotonicity is the property that makes resizing feel stable and
        // it cannot be satisfied by a lucky ordering.
        let mut previous: Option<(i32, u32)> = None;
        for width in (40..=1920).rev() {
            let Some(cluster) = utilities(bar(width as f32), 8.0, true) else {
                continue;
            };
            let score = richness(&cluster);
            if let Some((wider, richer)) = previous {
                assert!(
                    score <= richer,
                    "{width}px carries more ({score}) than the wider {wider}px did ({richer})"
                );
            }
            previous = Some((width, score));
            // Mute and the readout toggle travel together, never one alone.
            assert_eq!(
                cluster.mute.is_some(),
                cluster.readout.is_some(),
                "{width}px: mute and the readout toggle were shed separately"
            );
            // And the invariant the whole order exists to protect: the control that
            // gets a cramped window out of being cramped is never the one dropped.
            assert!(!cluster.fullscreen.is_empty(), "{width}px: no fullscreen");
        }
    }

    #[test]
    fn without_a_stream_the_cluster_carries_no_slider_at_any_width() {
        // A slider that writes nowhere is worse than an absent one: it invites a
        // drag and then does nothing, which reads as a broken control.
        for width in [200, 640, 960, 1280, 1920] {
            let cluster = utilities(bar(width as f32), 8.0, false).expect("wide enough");
            assert_eq!(
                cluster.volume, None,
                "{width}px drew a slider with no stream"
            );
            assert!(cluster.mute.is_some(), "{width}px: mute is still useful");
        }
    }

    #[test]
    fn a_bar_too_narrow_for_one_control_places_nothing() {
        assert_eq!(utilities(bar(CONTROL_SIZE - 1.0), 0.0, true), None);
        assert_eq!(utilities(bar(0.0), 0.0, true), None);
        assert_eq!(utilities(bar(f32::NAN), 0.0, true), None);
    }

    #[test]
    fn the_left_edge_is_always_left_of_every_control_it_reserves() {
        // `left_edge` is what the middle group is laid out against, so a value that
        // strayed right of a placed control would put the band's buttons under the
        // utilities — the exact overlap `timeline_layout` was written for.
        for width in 40..=1920 {
            let Some(cluster) = utilities(bar(width as f32), 8.0, true) else {
                continue;
            };
            for rect in [cluster.readout, cluster.mute, cluster.volume]
                .into_iter()
                .flatten()
                .chain(std::iter::once(cluster.fullscreen))
            {
                assert!(
                    cluster.left_edge <= rect.x + 0.01,
                    "{width}px: left_edge {} is right of a control at {}",
                    cluster.left_edge,
                    rect.x
                );
            }
        }
    }

    #[test]
    fn the_seek_group_is_three_boxes_or_nothing() {
        let group = seek_group(10.0, 4.0, 400.0).expect("400px seats the group");
        assert_eq!(group.start.width, CONTROL_SIZE);
        assert!(!group.start.overlaps(group.back));
        assert!(!group.back.overlaps(group.forward));
        assert!(group.start.x < group.back.x && group.back.x < group.forward.x);
        // Contiguous: the three read as one control, not three strays.
        assert!((group.back.x - (group.start.x + CONTROL_SIZE + CLUSTER_GAP)).abs() < 0.01);
        assert_eq!(seek_group(10.0, 4.0, 40.0), None);
    }

    #[test]
    fn the_position_bar_stays_inside_the_row_it_was_given() {
        // A sweep for the same reason the cluster has one: the interesting widths
        // are the ones where the middle group has just shed a control and the
        // leftover space changes discontinuously.
        for width in 40..=1920 {
            let bar = bar(width as f32);
            let Some(cluster) = utilities(bar, 8.0, true) else {
                continue;
            };
            let Some(scrub) = scrub_bar(bar.x + 40.0, cluster.left_edge, bar) else {
                continue;
            };
            for (name, rect) in [
                ("hit", scrub.hit),
                ("track", scrub.track),
                ("level", scrub.level),
            ] {
                assert!(
                    rect.x >= bar.x - 0.01 && rect.x + rect.width <= cluster.left_edge + 0.01,
                    "{width}px: the {name} rect leaves its span"
                );
                assert!(
                    rect.y >= bar.y - 0.01 && rect.y + rect.height <= bar.y + bar.height + 0.01,
                    "{width}px: the {name} rect leaves the row vertically"
                );
            }
            // The level is under the groove and never over it: a strip drawn
            // inside the seek fill would be the old meter again, one layer down.
            assert!(scrub.level.y >= scrub.track.y + scrub.track.height);
            assert!(!scrub.track.overlaps(scrub.level));
            assert!(scrub.hit.contains(scrub.track) && scrub.hit.contains(scrub.level));
        }
    }

    #[test]
    fn a_span_too_narrow_to_seek_in_places_no_bar() {
        let row = bar(1280.0);
        assert_eq!(scrub_bar(0.0, SCRUB_BAR_MIN_WIDTH - 1.0, row), None);
        assert!(scrub_bar(0.0, SCRUB_BAR_MIN_WIDTH, row).is_some());
        assert_eq!(scrub_bar(0.0, f32::NAN, row), None);
        // A row shorter than the bar cannot seat it either.
        assert_eq!(
            scrub_bar(0.0, 400.0, UiRect::new(0.0, 0.0, 400.0, 4.0)),
            None
        );
    }

    #[test]
    fn the_position_and_the_seek_target_are_inverses_of_each_other() {
        let track = UiRect::new(100.0, 0.0, 200.0, SCRUB_TRACK_HEIGHT);
        assert!((scrub_seconds(150.0, track, 120.0) - 30.0).abs() < 1e-9);
        assert!((scrub_fraction(30.0, 120.0) - 0.25).abs() < 1e-6);
        // Both ends, exactly.
        assert!((scrub_seconds(100.0, track, 120.0)).abs() < 1e-9);
        assert!((scrub_seconds(300.0, track, 120.0) - 120.0).abs() < 1e-9);
        // Past either end is a request for that end, not a refusal.
        assert!((scrub_seconds(-500.0, track, 120.0)).abs() < 1e-9);
        assert!((scrub_seconds(9000.0, track, 120.0) - 120.0).abs() < 1e-9);
        // An unmeasured stream reads empty rather than full: the meter this
        // replaced showed 22% at 0.15 s and users read it as position.
        assert_eq!(scrub_fraction(9.0, 0.0), 0.0);
        assert_eq!(scrub_fraction(9.0, f64::NAN), 0.0);
        assert_eq!(scrub_seconds(150.0, track, 0.0), 0.0);
        assert_eq!(scrub_fraction(f64::NAN, 120.0), 0.0);
    }

    #[test]
    fn the_modifier_ladder_matches_the_hint_it_is_drawn_beside() {
        assert!((seek_step_seconds(false, false) - 1.0).abs() < 1e-12);
        assert!((seek_step_seconds(true, false) - 0.1).abs() < 1e-12);
        assert!((seek_step_seconds(false, true) - 10.0).abs() < 1e-12);
        // Both held is not an error, and it is the fine step: overshooting by 10 s
        // when 0.1 s was meant is the more annoying of the two mistakes.
        assert!((seek_step_seconds(true, true) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn a_nudge_clamps_into_the_track_rather_than_refusing() {
        assert!((nudged(10.0, 1.0, 60.0) - 11.0).abs() < 1e-12);
        assert!((nudged(0.5, -1.0, 60.0) - 0.0).abs() < 1e-12);
        assert!((nudged(59.5, 10.0, 60.0) - 60.0).abs() < 1e-12);
        // An unknown duration must not pin the playhead at zero, which is what
        // clamping to a zero ceiling would do.
        assert!((nudged(10.0, 1.0, 0.0) - 11.0).abs() < 1e-12);
        assert!((nudged(10.0, 1.0, f64::NAN) - 11.0).abs() < 1e-12);
        // Garbage in stays inside the track rather than propagating.
        assert!((nudged(f64::NAN, 1.0, 60.0)).abs() < 1e-12);
        assert!((nudged(10.0, f64::INFINITY, 60.0)).abs() < 1e-12);
    }
}
