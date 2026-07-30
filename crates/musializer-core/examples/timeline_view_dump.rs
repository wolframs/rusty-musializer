//! Dumps the Rust timeline view over a wide grid of view states, strip
//! geometries and operation sequences, in the same format as
//! `tests/differential/timeline_view_oracle.c`.
//!
//! Run through `tools/differential_timeline_view.sh`, which builds
//! `timeline_view.c` from the frozen C source, runs both, and compares
//! numerically.
//!
//! Why this module gets a harness rather than a review: `ui::timeline_view` is the
//! single seconds↔pixel conversion the whole strip goes through — the tick ladder,
//! the playhead, the scrubber, the event markers, the lyric blocks and the
//! waveform envelope's per-column bin range all call into it. That is deliberate
//! (`timeline_view.h:6-11`) and it is also why a drift here is invisible: if the
//! view is wrong, every element on the strip is wrong *consistently*, so the strip
//! still photographs as a self-coherent picture.
//!
//! The port's own unit tests are almost entirely property assertions —
//! `is_finite()`, `span <= duration + 1e-9`, `span / step >= 2.0` — which a
//! genuinely wrong formula would survive. `../musializer/tests/test_timeline_view.c`
//! pins 32 exact values at 1e-9, and hand-transcribing those would risk a typo or,
//! worse, a number copied from our own output. A differential comparison cannot be
//! tautological.
//!
//! An `examples/` target rather than a `[[bin]]` on purpose: examples need no
//! manifest entry, so this does not touch a file another agent might be editing.
//!
//! The grid generator is duplicated between here and the C harness deliberately.
//! Sharing it would let a bug hide in the shared half. The grid *inputs* are
//! printed alongside the outputs so the two tables are themselves compared: if the
//! duplicated grids ever drift apart, the key columns fail before the value
//! columns do.

use std::io::{self, BufWriter, Write};

use musializer_core::ui::timeline_view::{
    tick_step, TimelineView, TIMELINE_VIEW_MIN_SPAN_SECONDS, TIMELINE_VIEW_REVEAL_MARGIN,
};

/// Seventeen digits after the point, in exponential form, matching the C harness.
/// A `f64` round-trips in seventeen significant digits, so printing is not the
/// limiting factor and any delta the comparison reports is arithmetic rather than
/// formatting. Exponential form also keeps a float token from ever looking like an
/// integer to the comparison script, which is how it knows which columns to
/// compare exactly.
///
/// Non-finite values get a *label* instead of `NaN`/`inf`. That is not cosmetic:
/// `nan` and `inf` both parse as floats in Python, and `abs(nan - nan) > tol` is
/// `False`, so a column of them would pass unconditionally — a silent hole the
/// project has already been bitten by once. These labels contain no digits and no
/// spelling `float()` accepts, so they fall through to the exact-string branch and
/// a NaN where a number belongs fails loudly. They are spelled by hand on both
/// sides rather than derived from either language's `Display`, which is what makes
/// C's `nan` and Rust's `NaN` a non-issue.
fn num(value: f64) -> String {
    if value.is_nan() {
        return "not_a_number".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "positive_infinity".to_string()
        } else {
            "negative_infinity".to_string()
        };
    }
    format!("{value:.17e}")
}

// ---------------------------------------------------------------------------
// The grid. Duplicated in timeline_view_oracle.c, value for value.
// ---------------------------------------------------------------------------

/// Durations for the clamp sweep. Zero, negative and non-finite are the "there is
/// no timeline" family; `1e-9` through `0.2500000001` straddle the span floor from
/// both sides, because a floor that does not yield to a very short track puts the
/// window past the end of the material (`timeline_view.c:11-17`).
#[rustfmt::skip]
const DURATIONS_A: &[f64] = &[
    0.0, -1.0, -240.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0e-9, 0.05, 0.1, 0.2499,
    0.25, 0.2500000001, 1.0, 3.7, 40.0, 90.0, 240.0, 600.0, 3600.0, 12345.678,
];

/// Raw view states fed straight to `clamp` without going through a mutator, which
/// is how a hot-reloaded layout or a caller that did its own arithmetic reaches it.
/// The non-finite ones exercise the recovery path (`timeline_view.c:36-41`), which
/// is the branch a property assertion is least able to distinguish from "happened
/// to be legal already".
#[rustfmt::skip]
const STATES_A: &[(f64, f64)] = &[
    (0.0, 0.0), (0.0, 240.0), (0.0, -5.0), (0.0, 0.25), (0.0, 0.1),
    (-30.0, 60.0), (30.0, 60.0), (230.0, 60.0), (1000.0, 60.0),
    (0.0, 1.0e-12), (0.0, 1.0e9), (120.0, 1.0e9),
    (f64::NAN, 60.0), (0.0, f64::NAN), (f64::NAN, f64::NAN), (f64::INFINITY, 60.0),
    (0.0, f64::INFINITY), (f64::NEG_INFINITY, 60.0), (0.0, f64::NEG_INFINITY),
    (239.9999999, 0.25),
];

/// Durations for the conversion sweep, including the unusable ones so
/// `seconds_at`'s `usable_duration` early return is covered.
const DURATIONS_B: &[f64] = &[-1.0, 0.0, 0.1, 1.0, 40.0, 240.0, 3600.0];

/// Strip geometries. A one-pixel and a two-pixel strip because the division by
/// width is where a narrow strip stops being representable; a negative left
/// because the strip is scrolled inside a panel; zero and negative width because a
/// panel can be laid out before it has been sized; non-finite because a NaN that
/// reaches a pixel conversion poisons every later frame.
#[rustfmt::skip]
const GEOMS_B: &[(f64, f64)] = &[
    (0.0, 1.0), (0.0, 2.0), (12.0, 1200.0), (-40.0, 900.0), (7.5, 1919.0),
    (12.0, 0.0), (12.0, -5.0), (12.0, f64::NAN), (f64::NAN, 900.0),
];

/// Moments to map onto the strip, deliberately including ones outside the window
/// and outside the track: `x_at` must report an off-screen edge honestly rather
/// than pre-clamping it (`timeline_view.h:58-61`).
#[rustfmt::skip]
const SECONDS_B: &[f64] = &[
    -10.0, 0.0, 1.0e-3, 0.05, 0.5, 3.7, 60.0, 120.0, 239.999, 240.0, 5000.0,
    f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
];

/// Pixel positions, in absolute coordinates rather than relative to `left`, so a
/// drag that leaves the strip is covered for every geometry.
#[rustfmt::skip]
const XS_B: &[f64] = &[
    -4000.0, -1.0, 0.0, 1.0, 12.0, 300.0, 611.5, 1212.0, 9000.0,
    f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
];

const DURATIONS_C: &[f64] = &[-1.0, 0.0, 0.1, 0.25, 1.0, 40.0, 240.0, 3600.0];

/// Zoom factors. `1e-9` and `1e12` hit the whole-track ceiling and the span floor
/// in one step; 0.9/1.0/1.1 sit either side of inert; 0.0, -2.0 and the non-finite
/// ones are the caller-bug family the C ignores outright.
#[rustfmt::skip]
const FACTORS_C: &[f64] = &[
    0.0, -2.0, f64::NAN, f64::INFINITY, 1.0e-9, 0.01, 0.5, 0.9, 1.0, 1.1, 2.0, 4.0, 64.0,
    1.0e12,
];

/// Anchors. The interesting ones are outside the current window and outside the
/// track entirely, because that is the case the header says is only violated when
/// honouring it would push the window off an end (`timeline_view.h:44-45`). A
/// non-finite anchor is not a caller bug but a request to zoom on the middle.
#[rustfmt::skip]
const ANCHORS_C: &[f64] = &[
    0.0, 0.05, 3.7, 20.0, 90.0, 120.0, 240.0, -50.0, 1.0e6, f64::NAN, f64::INFINITY,
];

/// Reveal targets and pan deltas, sharing one table because both are "a signed
/// amount of seconds the caller believes in". Both signs of a huge value are here
/// because reveal's two branches are asymmetric in the source — one subtracts the
/// margin, the other subtracts the span and adds it back
/// (`timeline_view.c:106-110`) — and a margin applied to the wrong side still keeps
/// the moment on screen.
#[rustfmt::skip]
const REVEALS_C: &[f64] = &[
    -1.0e9, -50.0, -0.001, 0.0, 0.05, 3.7, 20.0, 60.0, 120.0, 239.9, 240.0,
    1.0e6, f64::NAN, f64::INFINITY, f64::NEG_INFINITY,
];

/// The tick ladder, duplicated from `timeline_view.rs` so the boundary probes can
/// be generated from it. If the port's ladder differs, the `tick` rows disagree.
#[rustfmt::skip]
const LADDER: &[f64] = &[
    0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0,
    15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

/// Geometry used for the zoom and chain sections, where the point is the state
/// rather than the strip: the anchor's pixel position is what wheel zoom depends
/// on staying put, so it has to be measured somewhere.
const STRIP_LEFT: f64 = 12.0;
const STRIP_WIDTH: f64 = 1200.0;

// ---------------------------------------------------------------------------
// View constructors. Each one is a scripted sequence, so a state that only exists
// after two operations is reachable.
// ---------------------------------------------------------------------------

/// Nine starting states for the conversion sweep. Kinds 5 to 8 are deliberately
/// *not* clamped: `x_at` and `seconds_at` have their own guards and must survive a
/// state no mutator would have produced.
fn probe_view(kind: u32, duration: f64) -> TimelineView {
    let mut view = TimelineView {
        start_seconds: 0.0,
        span_seconds: 0.0,
    };
    match kind {
        0 => view.reset(duration),
        1 => {
            view.reset(duration);
            view.zoom(duration, 4.0, 90.0);
        }
        2 => {
            view.reset(duration);
            view.zoom(duration, 12.0, 77.0);
        }
        3 => {
            view.reset(duration);
            view.zoom(duration, 64.0, 200.0);
            view.pan(duration, 30.0);
        }
        4 => {
            view.reset(duration);
            view.zoom(duration, 1000.0, 10.0);
        }
        5 => {
            view.start_seconds = 0.0;
            view.span_seconds = f64::NAN;
        }
        6 => {
            view.start_seconds = 5.0;
            view.span_seconds = 0.0;
        }
        7 => {
            view.start_seconds = f64::NAN;
            view.span_seconds = 60.0;
        }
        _ => {
            view.start_seconds = -15.0;
            view.span_seconds = -3.0;
        }
    }
    view
}

/// Four starting states for the zoom sweep: whole, mid-track, near the floor at the
/// head, and pinned at the tail.
fn base_view(kind: u32, duration: f64) -> TimelineView {
    let mut view = TimelineView::new(duration);
    match kind {
        1 => view.zoom(duration, 4.0, 90.0),
        2 => view.zoom(duration, 200.0, 5.0),
        3 => view.zoom(duration, 8.0, duration),
        _ => {}
    }
    view
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

/// The two tuning constants, pinned directly. A harness that compares only derived
/// numbers would report a nudged floor as dozens of unexplained deltas; this row
/// names it.
fn dump_constants(out: &mut impl Write) {
    let _ = writeln!(
        out,
        "constant min_span {}",
        num(TIMELINE_VIEW_MIN_SPAN_SECONDS)
    );
    let _ = writeln!(
        out,
        "constant reveal_margin {}",
        num(TIMELINE_VIEW_REVEAL_MARGIN)
    );
}

fn dump_reset_and_clamp(out: &mut impl Write) {
    for &duration in DURATIONS_A {
        let view = TimelineView::new(duration);
        let _ = writeln!(
            out,
            "reset {} {} {} {}",
            num(duration),
            num(view.start_seconds),
            num(view.span_seconds),
            u8::from(view.is_whole(duration))
        );
    }

    for &duration in DURATIONS_A {
        for &(start, span) in STATES_A {
            let mut view = TimelineView {
                start_seconds: start,
                span_seconds: span,
            };
            // is_whole *before* the clamp, because the non-finite-span branch
            // answers true and is unreachable once clamp has run.
            let raw_whole = u8::from(view.is_whole(duration));
            view.clamp(duration);
            let _ = writeln!(
                out,
                "clamp {} {} {} {} {} {} {}",
                num(duration),
                num(start),
                num(span),
                raw_whole,
                num(view.start_seconds),
                num(view.span_seconds),
                u8::from(view.is_whole(duration))
            );
        }
    }
}

fn dump_conversions(out: &mut impl Write) {
    for &duration in DURATIONS_B {
        for kind in 0..9 {
            let view = probe_view(kind, duration);
            // The state itself, so a conversion difference can be told apart from
            // a difference in how the state was reached.
            let _ = writeln!(
                out,
                "probe {} {kind} {} {}",
                num(duration),
                num(view.start_seconds),
                num(view.span_seconds)
            );
            for &(left, width) in GEOMS_B {
                let _ = writeln!(
                    out,
                    "spp {} {kind} {} {}",
                    num(duration),
                    num(width),
                    num(view.seconds_per_pixel(width))
                );
                for &seconds in SECONDS_B {
                    let _ = writeln!(
                        out,
                        "x_at {} {kind} {} {} {} {}",
                        num(duration),
                        num(left),
                        num(width),
                        num(seconds),
                        num(view.x_at(seconds, left, width))
                    );
                }
                for &x in XS_B {
                    let _ = writeln!(
                        out,
                        "seconds_at {} {kind} {} {} {} {}",
                        num(duration),
                        num(left),
                        num(width),
                        num(x),
                        num(view.seconds_at(x, left, width, duration))
                    );
                }
                // The round trip the C's own test checks at 1e-6
                // (`tests/test_timeline_view.c:151-160`), here at every eleventh
                // of the strip for every geometry rather than one zoom level.
                for f in 0..=10 {
                    let fraction = f64::from(f) / 10.0;
                    let x = left + width * fraction;
                    let seconds = view.seconds_at(x, left, width, duration);
                    let back = view.x_at(seconds, left, width);
                    let _ = writeln!(
                        out,
                        "round_trip {} {kind} {} {} {} {} {}",
                        num(duration),
                        num(left),
                        num(width),
                        num(fraction),
                        num(seconds),
                        num(back)
                    );
                }
            }
        }
    }
}

fn dump_zoom(out: &mut impl Write) {
    for &duration in DURATIONS_C {
        for kind in 0..4 {
            for &factor in FACTORS_C {
                for &anchor in ANCHORS_C {
                    let mut view = base_view(kind, duration);
                    // Where the anchor sat before the zoom, and where it sits
                    // after. Wheel zoom feeling attached to the pointer is exactly
                    // these two numbers agreeing, and it is the property an
                    // off-by-a-fraction formulation still looks plausible without.
                    let before = view.x_at(anchor, STRIP_LEFT, STRIP_WIDTH);
                    view.zoom(duration, factor, anchor);
                    let after = view.x_at(anchor, STRIP_LEFT, STRIP_WIDTH);
                    let _ = writeln!(
                        out,
                        "zoom {} {kind} {} {} {} {} {} {} {}",
                        num(duration),
                        num(factor),
                        num(anchor),
                        num(before),
                        num(view.start_seconds),
                        num(view.span_seconds),
                        u8::from(view.is_whole(duration)),
                        num(after)
                    );
                }
            }
        }
    }
}

/// `reveal` and `pan` get their own sweep rather than only appearing inside the
/// chains, because between them they are the whole of follow-playback and of
/// dragging the strip, and a margin applied to the wrong edge still keeps the moment
/// on screen — which is all a property assertion can check.
fn dump_reveal_and_pan(out: &mut impl Write) {
    for &duration in DURATIONS_C {
        for kind in 0..4 {
            for &amount in REVEALS_C {
                let mut view = base_view(kind, duration);
                view.reveal(duration, amount);
                let _ = writeln!(
                    out,
                    "reveal {} {kind} {} {} {} {}",
                    num(duration),
                    num(amount),
                    num(view.start_seconds),
                    num(view.span_seconds),
                    u8::from(view.is_whole(duration))
                );

                let mut panned = base_view(kind, duration);
                panned.pan(duration, amount);
                let _ = writeln!(
                    out,
                    "pan {} {kind} {} {} {} {}",
                    num(duration),
                    num(amount),
                    num(panned.start_seconds),
                    num(panned.span_seconds),
                    u8::from(panned.is_whole(duration))
                );
            }
        }
    }
}

/// Follow-playback as it is actually used: `reveal` called once per frame while the
/// playhead advances across the whole track. This is the only shape in which the
/// margin's job — not re-centring on every frame at the boundary
/// (`timeline_view.h:23-25`) — is observable, because it depends on the *sequence*
/// of calls leaving the view alone until the playhead reaches the edge.
fn dump_follow(out: &mut impl Write) {
    for &duration in DURATIONS_C {
        for kind in 1..3 {
            let mut view = base_view(kind, duration);
            for step in 0..=97 {
                let playhead = duration * (f64::from(step) / 97.0);
                view.reveal(duration, playhead);
                let _ = writeln!(
                    out,
                    "follow {} {kind} {step} {} {} {} {} {}",
                    num(duration),
                    num(playhead),
                    num(view.start_seconds),
                    num(view.span_seconds),
                    u8::from(view.is_whole(duration)),
                    num(view.x_at(playhead, STRIP_LEFT, STRIP_WIDTH))
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sequences. The view is stateful, so a single-call harness would miss an error
// that only appears after two operations — a clamp that quietly shrinks the span,
// for instance, is invisible until the next pan reveals that the window got
// smaller.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Op {
    Reset,
    Clamp,
    Zoom(f64, f64),
    Pan(f64),
    Reveal(f64),
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Reset => "reset",
            Op::Clamp => "clamp",
            Op::Zoom(..) => "zoom",
            Op::Pan(_) => "pan",
            Op::Reveal(_) => "reveal",
        }
    }

    fn apply(self, view: &mut TimelineView, duration: f64) {
        match self {
            Op::Reset => view.reset(duration),
            Op::Clamp => view.clamp(duration),
            Op::Zoom(factor, anchor) => view.zoom(duration, factor, anchor),
            Op::Pan(delta) => view.pan(duration, delta),
            Op::Reveal(seconds) => view.reveal(duration, seconds),
        }
    }
}

#[rustfmt::skip]
const CHAINS: &[(&str, &[Op])] = &[
    ("zoom_in_then_out", &[
        Op::Zoom(2.0, 120.0), Op::Zoom(2.0, 120.0), Op::Zoom(2.0, 120.0),
        Op::Zoom(0.5, 120.0), Op::Zoom(0.5, 120.0), Op::Zoom(0.5, 120.0),
    ]),
    ("zoom_then_pan_tail", &[
        Op::Zoom(8.0, 0.0), Op::Pan(1000.0), Op::Pan(-3.0),
        Op::Zoom(2.0, f64::NAN), Op::Pan(-1.0e9),
    ]),
    ("reveal_follow", &[
        Op::Zoom(16.0, 0.0), Op::Reveal(10.0), Op::Reveal(20.0),
        Op::Reveal(30.0), Op::Reveal(200.0), Op::Reveal(0.0),
        Op::Reveal(f64::NAN),
    ]),
    ("floor_walk", &[
        Op::Zoom(3.0, 60.0), Op::Zoom(3.0, 60.0), Op::Zoom(3.0, 60.0),
        Op::Zoom(3.0, 60.0), Op::Zoom(3.0, 60.0), Op::Zoom(0.7, 60.0),
        Op::Zoom(0.7, 60.0), Op::Zoom(0.7, 60.0),
    ]),
    ("clamp_between", &[
        Op::Zoom(6.0, 50.0), Op::Clamp, Op::Pan(7.0),
        Op::Clamp, Op::Reveal(51.0), Op::Clamp,
    ]),
    ("nan_poison_recovery", &[
        Op::Zoom(4.0, 60.0), Op::Pan(f64::NAN), Op::Zoom(f64::NAN, 60.0),
        Op::Reveal(f64::INFINITY), Op::Clamp, Op::Zoom(2.0, 60.0),
    ]),
    ("reset_midway", &[
        Op::Zoom(32.0, 100.0), Op::Pan(50.0), Op::Reset,
        Op::Zoom(4.0, 10.0),
    ]),
    ("tail_anchor_zoom", &[
        Op::Zoom(4.0, 239.9), Op::Zoom(4.0, 239.9), Op::Zoom(4.0, 239.9),
        Op::Zoom(0.25, 239.9),
    ]),
    ("head_anchor_zoom", &[
        Op::Zoom(4.0, 0.0), Op::Zoom(4.0, 0.0), Op::Zoom(4.0, 0.0),
        Op::Zoom(0.25, 0.0),
    ]),
    ("negative_anchor", &[
        Op::Zoom(4.0, -100.0), Op::Zoom(4.0, -100.0), Op::Reveal(-5.0),
        Op::Pan(-2.0),
    ]),
    ("whole_track_ceiling", &[
        Op::Zoom(0.1, 120.0), Op::Zoom(0.1, 120.0), Op::Zoom(1.0e-9, 120.0),
        Op::Reveal(120.0),
    ]),
    // Panning must never change how much you can see. A clamp written as "shrink
    // the span to fit" keeps the invariant and silently zooms
    // (`tests/test_timeline_view.c:121-123`), which is only visible in a chain.
    ("pan_never_zooms", &[
        Op::Zoom(5.0, 77.0), Op::Pan(1.0e-3), Op::Pan(-1.0e-3),
        Op::Pan(1.0e6), Op::Pan(-1.0e6),
    ]),
];

const DURATIONS_D: &[f64] = &[0.0, 0.1, 3.7, 240.0];

fn dump_chains(out: &mut impl Write) {
    for &(name, ops) in CHAINS {
        for &duration in DURATIONS_D {
            let mut view = TimelineView::new(duration);
            for (step, &op) in ops.iter().enumerate() {
                op.apply(&mut view, duration);
                // State plus the three derived quantities the strip actually draws
                // from, so a state that is legal but shifted shows up.
                let _ = writeln!(
                    out,
                    "chain {name} {} {step} {} {} {} {} {} {} {}",
                    num(duration),
                    op.name(),
                    num(view.start_seconds),
                    num(view.span_seconds),
                    u8::from(view.is_whole(duration)),
                    num(view.seconds_per_pixel(STRIP_WIDTH)),
                    num(view.x_at(0.0, STRIP_LEFT, STRIP_WIDTH)),
                    num(view.x_at(duration, STRIP_LEFT, STRIP_WIDTH))
                );
            }
        }
    }
}

/// The tick ladder is a step function, so the only way to catch a rung being off
/// by one is to sweep densely enough to land on both sides of every boundary. The
/// boundary is at `span == 8 * rung` (`timeline_view.c:159-162`), so each rung is
/// probed exactly on it and a part in 1e12 either side, and then the whole range is
/// swept geometrically.
fn dump_tick_step(out: &mut impl Write) {
    for &rung in LADDER {
        let boundary = rung * 8.0;
        let probes = [
            boundary,
            boundary * (1.0 - 1.0e-12),
            boundary * (1.0 + 1.0e-12),
            rung,
            rung / 8.0,
        ];
        for probe in probes {
            let _ = writeln!(out, "tick {} {}", num(probe), num(tick_step(probe)));
        }
    }

    let mut span = 1.0e-4;
    while span < 1.0e5 {
        let _ = writeln!(out, "tick {} {}", num(span), num(tick_step(span)));
        span *= 1.02;
    }

    // The last entry is the smallest positive subnormal, spelled as `5e-324` on
    // both sides because that is the shortest literal that rounds to it: writing
    // its exact value out in full is more digits than a `f64` distinguishes, and
    // clippy is right to say so.
    #[rustfmt::skip]
    const DEGENERATE: &[f64] = &[
        0.0, -1.0, -1.0e300, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0e-300, 1.0e300,
        5.0e-324,
    ];
    for &span in DEGENERATE {
        let _ = writeln!(out, "tick {} {}", num(span), num(tick_step(span)));
    }
}

/// Divergences in mechanism, recorded rather than hidden. The C takes a bare
/// `Timeline_View *` and documents a null one as inert; Rust takes `&self` and
/// `&mut self`, so a null view is not a case it can refuse — it is a case it cannot
/// express. Both halves print their side of each pair and the driver asserts the
/// pair, which keeps the difference visible instead of silently excluded.
fn dump_divergences(out: &mut impl Write) {
    for label in [
        "null_view_x_at",
        "null_view_seconds_at",
        "null_view_is_whole",
        "null_view_seconds_per_pixel",
        "null_view_mutators",
    ] {
        let _ = writeln!(out, "divergence {label} not_expressible");
    }
}

fn main() {
    // Buffered: the grid is tens of thousands of lines, and an unbuffered
    // `println!` locks stdout once per line.
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    dump_constants(&mut out);
    dump_reset_and_clamp(&mut out);
    dump_conversions(&mut out);
    dump_zoom(&mut out);
    dump_reveal_and_pan(&mut out);
    dump_follow(&mut out);
    dump_chains(&mut out);
    dump_tick_step(&mut out);
    dump_divergences(&mut out);

    out.flush().expect("stdout");
}
