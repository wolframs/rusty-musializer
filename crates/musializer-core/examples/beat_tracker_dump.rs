//! Differential harness, Rust side: dumps [`BeatTracker`] over the same grid of
//! tick rates and onset patterns, and the same boundary cases, as
//! `tests/differential/beat_tracker_oracle.c`.
//!
//! Run through `tools/differential_beat_tracker.sh`, which builds both and
//! compares them.
//!
//! The generator below is a deliberate duplicate of the C harness's rather than a
//! shared fixture. A shared generator can hide the difference the comparison is
//! looking for, so the inputs are printed alongside the outputs and the two
//! generators are themselves part of what gets compared: if they drift apart, the
//! input columns fail before the value columns do. The reasoning for each case,
//! and for what is compared versus what is pinned behaviourally, is in the C
//! harness's header comment.
//!
//! This is an example rather than a test because examples need no manifest entry,
//! so a parallel agent adding one cannot collide with it.

use musializer_core::audio::beat_tracker::{BeatTracker, BeatUpdate};

/// Seventeen digits after the point, in exponential form: a `f64` round-trips in
/// seventeen significant digits, so any delta the comparison reports is
/// arithmetic rather than formatting, and an exponent keeps a float token from
/// ever looking like an integer to the comparison script.
///
/// Non-finite values get a label instead of `NaN`/`inf`, because both of those
/// spellings *parse* as floats in Python and `abs(nan - nan) > tol` is `false`, so
/// a column of them would pass unconditionally. That matters more here than in
/// most of these harnesses: feeding NaN in is one of the cases.
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

/// One update, and everything a caller can see afterwards.
///
/// The outcome column has three values, not two, and separating them is what this
/// harness found a parity bug with. The C's `beat_tracker_update` returns false
/// both when it refused the input (having written nothing) and when it computed a
/// phase that narrowed to exactly 1.0 (having written it) — and `plug.c:1139-1144`
/// uses whatever is in its local either way, so the observable results differ.
/// This port collapsed both into `None` until the harness compared them; the C
/// side tells them apart with a sentinel initialiser, which is the same mechanism
/// plug.c uses.
fn step(tracker: &mut BeatTracker, name: &str, index: i32, time: f64, onset: bool, strength: f32) {
    let update = tracker.update(time, onset, strength);
    let (outcome, value) = match update {
        BeatUpdate::Phase(phase) => ("phase", num(f64::from(phase))),
        BeatUpdate::OutOfRange(phase) => ("out_of_range", num(f64::from(phase))),
        BeatUpdate::Refused => ("refused", "not_reported".to_string()),
    };
    println!(
        "step {name} {index} {} {} {} {outcome} {value} {} {} {}",
        num(time),
        i32::from(onset),
        num(f64::from(strength)),
        num(tracker.interval_seconds()),
        tracker.learned_intervals(),
        i32::from(tracker.has_onset()),
    );
}

fn dump_reset(tracker: &mut BeatTracker, name: &str, index: i32) {
    tracker.reset();
    println!(
        "reset {name} {index} {} {} {}",
        num(tracker.interval_seconds()),
        tracker.learned_intervals(),
        i32::from(tracker.has_onset()),
    );
}

// ---------------------------------------------------------------------------
// The grid. Duplicated from beat_tracker_oracle.c, value for value.
// ---------------------------------------------------------------------------

/// A 5 ms analysis hop up to 0.8 s. The last is past the 0.75 s discontinuity
/// threshold, so that row of the grid restarts on every step and never learns
/// anything — a state a stalled preview really reaches, and the one where a
/// comparison slip hides because the tracker still returns a plausible phase.
#[rustfmt::skip]
const TICKS: [f64; 7] = [0.005, 1.0 / 60.0, 0.02, 0.037, 0.25, 0.3, 0.8];

/// `0` means "never fires". `1` means every step, which for the fast ticks is a
/// stream of sub-minimum gaps that must all be refused as observations.
#[rustfmt::skip]
const ONSET_PERIODS: [i32; 7] = [0, 1, 3, 8, 12, 24, 30];

/// Straddles the 0.04 noise floor from both sides, and includes a value far above
/// anything an onset detector produces.
#[rustfmt::skip]
const STRENGTHS: [f32; 8] = [0.0, 0.039, 0.04, 0.125, 0.5, 0.75, 1.0, 4.0];

/// Applied once per sequence, after the tick, to force a discontinuity mid-run.
/// The negative one drives time below zero for a few steps, so the rejection path
/// runs inside a live sequence and not only from a fresh tracker.
#[rustfmt::skip]
const JUMPS: [f64; 4] = [0.75, 0.7500000001, 5.0, -0.5];

const GRID_STEPS: i32 = 250;

/// A 64-bit LCG in integer arithmetic only, so the two sides cannot disagree
/// about the strength schedule for a floating-point reason. Knuth's MMIX
/// constants; nothing here needs a good generator, only an identical one.
fn lcg_next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

fn run_grid() {
    for (ti, tick) in TICKS.iter().enumerate() {
        for (pi, period) in ONSET_PERIODS.iter().enumerate() {
            let seq = ti * ONSET_PERIODS.len() + pi;
            let name = format!("grid_{ti}_{pi}");

            let mut tracker = BeatTracker::new();
            let mut lcg = 1u64.wrapping_add((seq as u64).wrapping_mul(2_654_435_761));
            let mut time = 0.0f64;
            let seek_step = 60 + (7 * seq) % 130;
            let jump = JUMPS[(ti + pi) % JUMPS.len()];

            for index in 0..GRID_STEPS {
                // Drawn unconditionally so the two streams stay aligned even
                // where the value is unused.
                let strength = STRENGTHS[(lcg_next(&mut lcg) % STRENGTHS.len() as u64) as usize];
                let onset = *period != 0 && index % *period == 0;
                step(&mut tracker, &name, index, time, onset, strength);
                time += *tick;
                if index as usize == seek_step {
                    time += jump;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Boundary cases. Literal times throughout — no arithmetic at all — so that
// "exactly 0.75" means the double nearest 0.75 on both sides rather than
// whatever an accumulator happened to land on.
// ---------------------------------------------------------------------------

/// `(time, onset, strength)`.
type Input = (f64, bool, f32);

fn run_case(name: &str, inputs: &[Input]) {
    let mut tracker = BeatTracker::new();
    for (index, &(time, onset, strength)) in inputs.iter().enumerate() {
        step(&mut tracker, name, index as i32, time, onset, strength);
    }
}

/// The restart test is `> 0.75`, so a gap of exactly 0.75 must not restart and one
/// double above it must. Nothing in either test suite uses a 0.75 gap.
#[rustfmt::skip]
const RESTART_BOUNDARY: [Input; 7] = [
    (0.0,             false, 0.0),
    (0.75,            false, 0.0),
    (1.5,             false, 0.0),
    (2.2500000001,    false, 0.0),
    (3.0,             false, 0.0),
    (3.75,            false, 0.0),
    (4.5000000000001, false, 0.0),
];

/// An inter-onset gap of exactly 0.25 s is credible; one double below is not.
/// Both are reachable only because the tracker is ticked in between, which keeps
/// `previous_time` close enough to avoid a restart.
#[rustfmt::skip]
const GAP_MIN_BOUNDARY: [Input; 10] = [
    (0.0,                false, 0.0),
    (0.0,                true,  0.5),
    (0.125,              false, 0.0),
    (0.25,               true,  0.5),
    (0.375,              false, 0.0),
    (0.5,                true,  0.5),
    (0.625,              false, 0.0),
    (0.7499999999999999, true,  0.5),
    (0.875,              false, 0.0),
    (1.0,                true,  0.5),
];

/// An inter-onset gap of exactly 1.5 s is credible; one double above is refused
/// *entirely* — it must not even move the anchor, so the phase keeps running off
/// the previous one. "Ignored, not clamped" is the easiest thing here to get wrong.
///
/// The digits on the last onset are the case, not noise: it is `4.5` plus one ULP,
/// which makes the gap from the onset at `3.0` one hair over the 1.5 s limit. The C
/// harness spells it the same way, and keeping the two textually identical is what
/// makes the duplicated tables auditable — so the precision lint is allowed here
/// rather than the digits being trimmed.
#[rustfmt::skip]
#[allow(clippy::excessive_precision)]
const GAP_MAX_BOUNDARY: [Input; 12] = [
    (0.0,                true,  0.5),
    (0.5,                false, 0.0),
    (1.0,                false, 0.0),
    (1.5,                true,  0.5),
    (2.0,                false, 0.0),
    (2.5,                false, 0.0),
    (3.0,                true,  0.5),
    (3.5,                false, 0.0),
    (4.0,                false, 0.0),
    (4.5000000000000009, true,  0.5),
    (5.0,                false, 0.0),
    (5.5,                false, 0.0),
];

/// 0.04 is the floor and counts; the nearest float below it does not. `-0.0` is
/// here because `strength < 0.0` is false for it in both languages, so it is
/// accepted as a zero rather than refused as a negative.
///
/// A function rather than a `const` table, matching the C side, because the value
/// just below the floor has to be computed. Writing it as a decimal does not work:
/// `0.0399999991` rounds back *to* `0.04f32` and silently duplicates the row above
/// it, so the boundary would look tested and not be — clippy's precision lint is
/// what caught that. `f32::from_bits` is only `const` from Rust 1.83 and this
/// workspace's MSRV is 1.80, so it is a `let`. The C gets the identical value from
/// `nextafterf(0.04f, 0.0f)`.
fn run_strength_floor() {
    let just_below = f32::from_bits(0.04f32.to_bits() - 1);
    #[rustfmt::skip]
    let inputs: [Input; 10] = [
        (0.0,  true,  0.0),
        (0.1,  true,  -0.0),
        (0.2,  true,  0.039),
        (0.3,  true,  just_below),
        (0.4,  true,  0.04),
        (0.5,  false, 0.0),
        (0.65, true,  just_below),
        (0.8,  true,  0.04),
        (0.9,  false, 0.0),
        (1.05, true,  1.0),
    ];
    run_case("strength_floor", &inputs);
}

/// Drives the estimate towards 0.6 s, then feeds a half-interval and a
/// double-interval observation. Both must fold into agreement rather than halving
/// or doubling the reported tempo.
#[rustfmt::skip]
const TEMPO_FOLD: [Input; 22] = [
    (0.0,  true,  0.5),
    (0.3,  false, 0.0),
    (0.6,  true,  0.5),
    (0.9,  false, 0.0),
    (1.2,  true,  0.5),
    (1.5,  false, 0.0),
    (1.8,  true,  0.5),
    (2.1,  false, 0.0),
    (2.4,  true,  0.5),
    (2.7,  false, 0.0),
    (3.0,  true,  0.5),
    (3.3,  false, 0.0),
    (3.6,  true,  0.5),
    (3.9,  false, 0.0),
    // half the running estimate: folds up
    (4.2,  false, 0.0),
    (4.5,  true,  0.5),
    (4.8,  false, 0.0),
    // roughly double: folds down
    (5.4,  true,  0.5),
    (5.7,  false, 0.0),
    (6.0,  false, 0.0),
    (6.6,  true,  0.5),
    (6.9,  false, 0.0),
];

/// Five exact 0.5 s gaps take `learned_intervals` from 0 through 5, so the blend
/// weight changes from 0.42 to 0.20 mid-case and the 0.7 s outlier at the end is
/// absorbed by whichever weight is then current. Two separate primings would let a
/// wrong threshold pass by being wrong twice.
#[rustfmt::skip]
const WEIGHT_TRANSITION: [Input; 10] = [
    (0.0, true, 0.5),
    (0.5, true, 0.5),
    (1.0, true, 0.5),
    (1.5, true, 0.5),
    (2.0, true, 0.5),
    (2.5, true, 0.5),
    (3.0, true, 0.5),
    (3.7, true, 0.5),
    (4.2, true, 0.5),
    (4.7, true, 0.5),
];

/// A rejection with no invalid input.
///
/// `position` is `0.99999999999` as an `f64`, inside `[0, 1)`; narrowed to `f32` it
/// becomes exactly `1.0` and the final check refuses it — and reports it, which is
/// the difference this case exists to pin.
///
/// Written as a deliberate case, but it turns out not to be a corner: the grid
/// above reaches the same state 187 times without trying, because accumulated tick
/// drift puts the position a hair under an exact multiple of the interval whenever
/// the tick nearly divides it. Neither test suite has a case for it.
#[rustfmt::skip]
const FLOAT_ROUNDS_TO_ONE: [Input; 3] = [
    (0.0,            false, 0.0),
    (0.499999999995, false, 0.0),
    (0.5,            false, 0.0),
];

#[rustfmt::skip]
const REJECT_FRESH: [Input; 2] = [
    (-1.0, false, 0.0),
    (-0.0, false, 0.0),   // not negative: accepted
];

/// Every input the tracker refuses, from a running tracker, followed by a valid
/// call proving the refusals left no residue.
///
/// The huge finite time is not padding: `1e18 / 0.5` is a whole `f64`, so the
/// fractional part is exactly zero and the phase reads 0 — and the next small time
/// is then a backwards jump, which restarts.
fn run_rejections() {
    run_case("reject_fresh", &REJECT_FRESH);

    let mut tracker = BeatTracker::new();
    let mut index = 0;
    let mut next = |tracker: &mut BeatTracker, time: f64, onset: bool, strength: f32| {
        step(tracker, "reject_running", index, time, onset, strength);
        index += 1;
    };
    next(&mut tracker, 0.0, true, 0.5);
    next(&mut tracker, 0.5, true, 0.5);
    next(&mut tracker, f64::NAN, false, 0.0);
    next(&mut tracker, f64::INFINITY, false, 0.0);
    next(&mut tracker, f64::NEG_INFINITY, false, 0.0);
    next(&mut tracker, -1.0e-300, false, 0.0);
    next(&mut tracker, 1.0, false, f32::NAN);
    next(&mut tracker, 1.0, false, f32::INFINITY);
    next(&mut tracker, 1.0, false, f32::NEG_INFINITY);
    next(&mut tracker, 1.0, false, -0.1);
    // Normal rather than subnormal on purpose: a decimal literal in the subnormal
    // range is a needless bet that both compilers round it the same way, and the
    // thing under test is the sign check.
    next(&mut tracker, 1.0, false, -1.0e-30);
    // Nothing above touched the state, so this reads as if it came straight after
    // the 0.5 s onset.
    next(&mut tracker, 1.0, false, 0.5);
    next(&mut tracker, 1.0e18, false, 0.5);
    next(&mut tracker, 2.0, false, 0.5);
    // The C side writes this unsuffixed for a reason recorded there: as
    // `3.4028235e38f` it was a float literal widened to the double parameter, a
    // different number from the same decimal parsed as a double, and the two
    // generators disagreed by 3.4e30 on the first run. The echoed input columns are
    // what caught it.
    next(&mut tracker, 1.0e300, false, 0.5);
}

/// `reset` is the public "forget everything", and deliberately *not* the internal
/// restart a discontinuity performs: the restart keeps a credible interval across
/// the gap, `reset` drops it. Both are dumped from the same learned state so the
/// difference is a pair of rows rather than a claim.
fn run_reset_versus_seek() {
    let mut tracker = BeatTracker::new();
    let mut index = 0;
    for beat in 0..6 {
        step(
            &mut tracker,
            "reset_vs_seek",
            index,
            f64::from(beat) * 0.6,
            true,
            0.5,
        );
        index += 1;
    }
    // A forward jump past 0.75 s: new phase reference, tempo survives.
    step(&mut tracker, "reset_vs_seek", index, 30.0, false, 0.0);
    index += 1;
    step(&mut tracker, "reset_vs_seek", index, 30.25, false, 0.0);
    index += 1;
    // Now the explicit reset from the same shape of state.
    dump_reset(&mut tracker, "reset_vs_seek", index);
    index += 1;
    step(&mut tracker, "reset_vs_seek", index, 30.5, false, 0.0);
    index += 1;
    step(&mut tracker, "reset_vs_seek", index, 30.75, false, 0.0);
}

fn main() {
    run_grid();

    run_case("restart_boundary", &RESTART_BOUNDARY);
    run_case("gap_min_boundary", &GAP_MIN_BOUNDARY);
    run_case("gap_max_boundary", &GAP_MAX_BOUNDARY);
    run_strength_floor();
    run_case("tempo_fold", &TEMPO_FOLD);
    run_case("weight_transition", &WEIGHT_TRANSITION);
    run_case("float_rounds_to_one", &FLOAT_ROUNDS_TO_ONE);
    run_rejections();
    run_reset_versus_seek();

    // Mechanism divergences, stated rather than quietly excluded. Both are inputs
    // this side cannot express: C takes bare pointers and documents a null tracker
    // or null out-parameter as a refusal, and `&mut self` plus a returned enum are
    // not things a caller can hand a null.
    //
    // There used to be a third entry claiming the C's out-of-range write was a
    // quirk this port deliberately did not reproduce. That was wrong — the C's
    // caller uses the written value — and writing the claim down is what exposed
    // it. It is now a compared column on every `step` row instead.
    println!("divergence null_tracker not_expressible");
    println!("divergence null_phase_out not_expressible");
}
