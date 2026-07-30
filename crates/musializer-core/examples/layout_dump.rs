//! Dumps the Rust workspace and timeline layout decisions over a dense sweep of
//! window sizes, in the same format as `tests/differential/layout_oracle.c`.
//!
//! Run through `tools/differential_layout.sh`, which builds the oracle's two
//! layout modules from the frozen C source, runs both, and compares numerically.
//!
//! Why these two modules earn a harness: their ported tests are mostly *property*
//! assertions ("this rect is inside that one"), and a property survives a
//! genuinely wrong formula. A layout drift is also nearly invisible in a
//! screenshot, because everything moves together and still looks plausible.
//!
//! An `examples/` target rather than a `[[bin]]` on purpose: examples need no
//! manifest entry, so this does not touch a file another agent might be editing.
//!
//! The case generator is duplicated between here and the C harness deliberately —
//! the same arithmetic written twice. Sharing it would let a bug hide in the
//! shared half. Every case's *inputs* are printed as compared columns too, so the
//! two copies drifting apart fails the harness rather than silently comparing
//! different sweeps.
//!
//! Rejected calls print the sentinel the C's out-parameter was pre-filled with,
//! so C's "returns false and leaves `*out` untouched" is a compared column. Here
//! the guarantee is structural (`Option`), and printing the same sentinel is what
//! pairs the two.

use musializer_core::ui::timeline_layout::{
    TimelineBand, TIMELINE_BAND_GAP, TIMELINE_BAND_MIN_SCALE,
};
use musializer_core::ui::workspace_layout::{
    TracksPanelMode, UiRect, WorkspaceSidebar, WORKSPACE_SCENES_MAXIMUM, WORKSPACE_SCENES_MINIMUM,
    WORKSPACE_TRACKS_ITEM_RATIO, WORKSPACE_TRACKS_MAXIMUM, WORKSPACE_TRACKS_SINGLE_MINIMUM,
    WORKSPACE_TRACKS_STACKED_HEADER, WORKSPACE_TRACKS_STACKED_MINIMUM,
};

/// What the C harness pre-fills every out-parameter with.
const SENTINEL_F: f32 = -12345.0;
/// Not a valid `Tracks_Panel_Mode`, so "the C did not write" is visible.
const SENTINEL_MODE: i32 = 7;

/// Ten significant digits, matching the C's `%.9e`. That round-trips an IEEE
/// single exactly, so the comparison is effectively bit-exact.
fn f(value: f32) -> String {
    format!("{value:.9e}")
}

fn rect_fields(rect: UiRect) -> String {
    format!(
        "{} {} {} {}",
        f(rect.x),
        f(rect.y),
        f(rect.width),
        f(rect.height)
    )
}

const SENTINEL_RECT: UiRect = UiRect::new(SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F);

// ---------------------------------------------------------------------------
// UiRect helpers: is_finite / is_empty / contains / overlaps / intersect.
//
// Swept as every ordered pair, both ways round, because contains and intersect
// are asymmetric and the empty/non-finite refusals are exactly where a port
// drifts. The non-finite entries are also what exercises the comparator's
// non-finite path, which AGENTS.md records as a hole a previous harness fell
// into.
// ---------------------------------------------------------------------------

fn rect_table() -> Vec<UiRect> {
    vec![
        UiRect::new(0.0, 0.0, 100.0, 100.0),
        UiRect::new(10.0, 10.0, 50.0, 50.0),
        UiRect::new(-1.0, 0.0, 10.0, 10.0),
        UiRect::new(0.0, 95.0, 10.0, 10.0),
        UiRect::new(50.0, 50.0, 10.0, 10.0),
        UiRect::new(0.0, 0.0, 10.0, 10.0),
        UiRect::new(5.0, 5.0, 10.0, 10.0),
        // Shares an edge.
        UiRect::new(0.0, 10.0, 10.0, 10.0),
        UiRect::new(0.0, 0.0, 10.0, 0.0),
        UiRect::new(0.0, 0.0, 0.0, 10.0),
        UiRect::new(0.0, 0.0, -5.0, 10.0),
        UiRect::new(0.0, 0.0, 10.0, -5.0),
        // A single-mode action row.
        UiRect::new(10.0, 50.0, 52.0, 36.0),
        // The zero-height panel that stole scene clicks.
        UiRect::new(0.0, 0.0, 240.0, 0.0),
        // Tracks and scenes at a 460 px sidebar.
        UiRect::new(0.0, 0.0, 240.0, 188.0),
        UiRect::new(0.0, 188.0, 240.0, 272.0),
        UiRect::new(0.0, 0.0, 320.0, 168.0),
        UiRect::new(0.0, 168.0, 320.0, 355.0),
        UiRect::new(f32::NAN, 10.0, 10.0, 10.0),
        UiRect::new(0.0, f32::NAN, 10.0, 10.0),
        UiRect::new(0.0, 0.0, f32::NAN, 10.0),
        UiRect::new(0.0, 0.0, 10.0, f32::NAN),
        UiRect::new(0.0, 0.0, f32::INFINITY, 10.0),
        UiRect::new(0.0, 0.0, 10.0, f32::INFINITY),
        UiRect::new(f32::NEG_INFINITY, 0.0, 10.0, 10.0),
        // x + width overflows to +inf in f32, so the containment arithmetic
        // itself goes non-finite while every field is finite.
        UiRect::new(1.0e38, 1.0e38, 1.0e38, 1.0e38),
        UiRect::new(-1.0e38, 0.0, 2.0e38, 10.0),
        UiRect::new(0.5, 0.25, 99.5, 99.75),
        UiRect::new(100.0, 100.0, 0.0625, 0.0625),
    ]
}

fn dump_rect_helpers(out: &mut String) {
    let rects = rect_table();
    for (i, rect) in rects.iter().enumerate() {
        out.push_str(&format!("rect_table {i} {}\n", rect_fields(*rect)));
    }
    for (i, a) in rects.iter().enumerate() {
        for (j, b) in rects.iter().enumerate() {
            let ab = a.intersect(*b);
            let ba = b.intersect(*a);
            out.push_str(&format!(
                "rect {i} {j} {} {} {} {} {} {} {} {} {} {}\n",
                a.is_finite() as i32,
                a.is_empty() as i32,
                b.is_finite() as i32,
                b.is_empty() as i32,
                a.contains(*b) as i32,
                b.contains(*a) as i32,
                a.overlaps(*b) as i32,
                b.overlaps(*a) as i32,
                rect_fields(ab),
                rect_fields(ba),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// TracksPanelMode::action_row.
// ---------------------------------------------------------------------------

fn mode_int(mode: TracksPanelMode) -> i32 {
    // Explicit rather than `as i32`, because the pairing with C's enum is a
    // contract and not an accident of declaration order.
    match mode {
        TracksPanelMode::Hidden => 0,
        TracksPanelMode::Single => 1,
        TracksPanelMode::Stacked => 2,
    }
}

fn dump_action_rows(out: &mut String) {
    for mode in [
        TracksPanelMode::Hidden,
        TracksPanelMode::Single,
        TracksPanelMode::Stacked,
    ] {
        let (ok, top, height) = match mode.action_row() {
            // C leaves both out-parameters untouched when it returns false, so
            // the sentinel is what pairs with `None` here.
            None => (0, SENTINEL_F, SENTINEL_F),
            Some((top, height)) => (1, top, height),
        };
        out.push_str(&format!(
            "action_row {} {ok} {} {}\n",
            mode_int(mode),
            f(top),
            f(height)
        ));
    }
}

// ---------------------------------------------------------------------------
// WorkspaceSidebar::layout.
// ---------------------------------------------------------------------------

// Duplicated, on purpose, from layout_oracle.c.
const SIDEBAR_DENSE_WIDTHS: [f32; 2] = [168.0, 320.0];
const SIDEBAR_DENSE_COUNTS: [usize; 4] = [0, 1, 3, 12];
const SIDEBAR_GRID_WIDTHS: [f32; 11] = [
    1.0, 2.0, 96.0, 168.0, 200.0, 240.0, 280.0, 320.0, 400.0, 640.0, 3000.0,
];
const SIDEBAR_GRID_COUNTS: [usize; 11] = [0, 1, 2, 3, 4, 5, 8, 12, 64, 512, 4096];
/// Sixteenths, so a threshold-adjacent probe is exactly representable and the two
/// generators cannot disagree about what "one pixel below" means.
const THRESHOLD_OFFSETS: [f32; 5] = [-1.0, -0.0625, 0.0, 0.0625, 1.0];

/// The window grid: the sizes a user can actually produce, plus the degenerate
/// and absurd ends. 960x640 is the documented minimum and 1280x720 the default.
const WINDOW_WIDTHS: [f32; 13] = [
    0.0, 1.0, 200.0, 640.0, 960.0, 1024.0, 1280.0, 1366.0, 1600.0, 1920.0, 2560.0, 3840.0, 7680.0,
];
const WINDOW_HEIGHTS: [f32; 14] = [
    0.0, 1.0, 300.0, 480.0, 600.0, 640.0, 720.0, 768.0, 800.0, 900.0, 1080.0, 1440.0, 2160.0,
    4320.0,
];

/// The rail width the shell hands the sidebar. Plain arithmetic, duplicated.
// Sequential ifs, not `clamp`, so this is the same expression the C harness runs:
// `clamp` propagates NaN differently and would make the window grid's degenerate
// entries diverge for a reason that has nothing to do with the port.
#[allow(clippy::manual_clamp)]
fn rail_width(window_width: f32) -> f32 {
    let mut rail = window_width * 0.25;
    if rail < 240.0 {
        rail = 240.0;
    }
    if rail > 320.0 {
        rail = 320.0;
    }
    rail
}

struct Sidebars {
    out: String,
    index: usize,
}

impl Sidebars {
    fn emit(&mut self, width: f32, height: f32, track_count: usize) {
        let (ok, tracks, scenes, mode) = match WorkspaceSidebar::layout(width, height, track_count)
        {
            None => (0, SENTINEL_RECT, SENTINEL_RECT, SENTINEL_MODE),
            Some(layout) => (
                1,
                layout.tracks,
                layout.scenes,
                mode_int(layout.tracks_mode),
            ),
        };
        self.out.push_str(&format!(
            "sidebar {} {} {} {track_count} {ok} {} {} {mode}\n",
            self.index,
            f(width),
            f(height),
            rect_fields(tracks),
            rect_fields(scenes),
        ));
        self.index += 1;
    }
}

fn dump_sidebars(out: &mut String) {
    let mut sink = Sidebars {
        out: String::new(),
        index: 0,
    };

    // Dense: one pixel at a time through every threshold, at the rail width the
    // shell produces with no inspector.
    for count in SIDEBAR_DENSE_COUNTS {
        for h in 1..=1400 {
            sink.emit(240.0, h as f32, count);
        }
    }
    // Dense, coarser, at the two other rail widths the shell can produce.
    for width in SIDEBAR_DENSE_WIDTHS {
        for count in SIDEBAR_DENSE_COUNTS {
            let mut h = 1;
            while h <= 1400 {
                sink.emit(width, h as f32, count);
                h += 7;
            }
        }
    }
    // Threshold-adjacent: the whole point. Every boundary the module compares
    // against, probed a sixteenth and a pixel either side.
    for width in SIDEBAR_GRID_WIDTHS {
        for count in SIDEBAR_GRID_COUNTS {
            let mut wanted = WORKSPACE_TRACKS_STACKED_HEADER
                + count as f32 * width * WORKSPACE_TRACKS_ITEM_RATIO;
            if !wanted.is_finite() || wanted < WORKSPACE_TRACKS_STACKED_MINIMUM {
                wanted = WORKSPACE_TRACKS_STACKED_MINIMUM;
            }
            if wanted > WORKSPACE_TRACKS_MAXIMUM {
                wanted = WORKSPACE_TRACKS_MAXIMUM;
            }
            let bases = [
                wanted,
                wanted + WORKSPACE_SCENES_MINIMUM,
                wanted + WORKSPACE_SCENES_MAXIMUM,
                wanted + WORKSPACE_TRACKS_SINGLE_MINIMUM,
                wanted + WORKSPACE_TRACKS_STACKED_MINIMUM,
                WORKSPACE_SCENES_MINIMUM,
                WORKSPACE_SCENES_MAXIMUM,
                WORKSPACE_TRACKS_SINGLE_MINIMUM,
                WORKSPACE_TRACKS_STACKED_MINIMUM,
            ];
            for base in bases {
                for offset in THRESHOLD_OFFSETS {
                    sink.emit(width, base + offset, count);
                }
            }
        }
    }
    // The window grid, twice: once treating the window as the sidebar directly
    // (extremely wide and extremely tall aspect ratios, and the 0 cases), once
    // through the rail/timeline derivation the shell actually performs.
    for width in WINDOW_WIDTHS {
        for height in WINDOW_HEIGHTS {
            for count in [0usize, 1, 3] {
                sink.emit(width, height, count);
                sink.emit(rail_width(width), height - 180.0, count);
            }
        }
    }
    // Extremes and non-finite input. The huge counts are what reach the oracle's
    // "an overflowed ask goes to the *minimum*" branch, which the port kept.
    #[rustfmt::skip]
    let cases: [(f32, f32, usize); 24] = [
        (240.0,        0.0,     1),
        (240.0,       -10.0,    1),
        (0.0,          600.0,   1),
        (-1.0,         600.0,   1),
        (1.0e-3,       1.0e-3,  0),
        (0.0625,       640.0,   0),
        (240.0,        1.0e-3,  1),
        (1.0e38,       600.0,   1),
        (3.0e38,       600.0,   1),
        (240.0,        1.0e38,  1),
        (240.0,        3.0e38,  1),
        (240.0,        600.0,   usize::MAX),
        (1.0e38,       600.0,   usize::MAX),
        (1.0e38,       1.0e38,  usize::MAX),
        (3.0e38,       3.0e38,  usize::MAX),
        (240.0,        600.0,   1usize << 40),
        (1.0e20,       600.0,   1usize << 40),
        (f32::NAN,     600.0,   1),
        (240.0,        f32::NAN, 1),
        (f32::NAN,     f32::NAN, 1),
        (f32::INFINITY, 600.0,  1),
        (240.0, f32::INFINITY,  1),
        (f32::NEG_INFINITY, 600.0, 1),
        (240.0, f32::NEG_INFINITY, 1),
    ];
    for (width, height, count) in cases {
        sink.emit(width, height, count);
    }

    out.push_str(&sink.out);
}

// ---------------------------------------------------------------------------
// TimelineBand::layout.
//
// The parameterised variants matter here: the control row has a different
// membership in the transport bar than in the event panel, and the trailing
// clear button is present in one and zero-width in the other. Those combinations
// are part of the contract, so the sweep drives all of them.
// ---------------------------------------------------------------------------

/// Duplicated from `layout_oracle.c`'s `control_sets`. The C pads each row to a
/// fixed width and carries the real length beside it; a slice needs no padding,
/// so the lengths are implicit here and printed as a compared column.
#[rustfmt::skip]
const CONTROL_SETS: [&[f32]; 9] = [
    &[112.0],
    // The event panel's marker row (`panels/events.rs` geometry).
    &[74.0, 82.0, 86.0],
    // The shipped panel/event row the ported tests use.
    &[74.0, 74.0, 90.0, 74.0, 82.0, 86.0],
    // Exactly TIMELINE_BAND_CONTROL_CAPACITY.
    &[40.0, 40.0, 40.0, 40.0, 40.0, 40.0, 40.0, 40.0],
    // Zero widths: natural comes only from the margins and the clear button.
    &[0.0, 0.0, 0.0],
    &[220.0, 96.0],
    &[33.5, 47.25, 61.0625, 12.5],
    // The transport bar's six buttons, rounded to what the measurer yields.
    &[64.0, 64.0, 64.0, 64.0, 64.0],
    // One past capacity: both sides can express this rejection.
    &[40.0, 40.0, 40.0, 40.0, 40.0, 40.0, 40.0, 40.0, 40.0],
];
const CONTROL_CLEARS: [f32; 9] = [112.0, 112.0, 112.0, 0.0, 112.0, 0.0, 18.5, 0.0, 112.0];

const BAND_MARGINS: [f32; 3] = [0.0, 6.0, 12.0];
const BAND_TIMECODES: [f32; 5] = [0.0, 90.0, 145.0, 232.0, 400.0];

struct Timelines {
    out: String,
    index: usize,
}

impl Timelines {
    #[allow(clippy::too_many_arguments)]
    fn emit_widths(
        &mut self,
        band_x: f32,
        band_y: f32,
        band_width: f32,
        band_height: f32,
        margin: f32,
        set_label: usize,
        control_widths: &[f32],
        clear_width: f32,
        timecode_width: f32,
    ) {
        let result = TimelineBand::layout(
            band_x,
            band_y,
            band_width,
            band_height,
            margin,
            control_widths,
            clear_width,
            timecode_width,
        );
        let (ok, scale, controls_width, controls, clear, timecode, inline, fits) = match result {
            None => (
                0,
                SENTINEL_F,
                SENTINEL_F,
                SENTINEL_RECT,
                SENTINEL_RECT,
                SENTINEL_RECT,
                0,
                0,
            ),
            Some(band) => (
                1,
                band.scale,
                band.controls_width,
                band.controls,
                band.clear,
                band.timecode,
                band.timecode_inline as i32,
                band.fits as i32,
            ),
        };
        self.out.push_str(&format!(
            "timeline {} {} {} {} {} {} {set_label} {} {} {} {ok} {} {} {} {} {} {inline} {fits}\n",
            self.index,
            f(band_x),
            f(band_y),
            f(band_width),
            f(band_height),
            f(margin),
            control_widths.len(),
            f(clear_width),
            f(timecode_width),
            f(scale),
            f(controls_width),
            rect_fields(controls),
            rect_fields(clear),
            rect_fields(timecode),
        ));
        self.index += 1;
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        band_x: f32,
        band_y: f32,
        band_width: f32,
        band_height: f32,
        margin: f32,
        set: usize,
        clear_width: f32,
        timecode_width: f32,
    ) {
        self.emit_widths(
            band_x,
            band_y,
            band_width,
            band_height,
            margin,
            set,
            CONTROL_SETS[set],
            clear_width,
            timecode_width,
        );
    }
}

/// The row's unscaled extent, recomputed here rather than read back out of the
/// result, so the threshold probes do not depend on the thing under test.
fn natural_width(set: usize, margin: f32, clear_width: f32) -> f32 {
    let mut natural = clear_width;
    for &width in CONTROL_SETS[set] {
        natural += width + margin;
    }
    natural
}

fn dump_timelines(out: &mut String) {
    for (set, widths) in CONTROL_SETS.iter().enumerate() {
        for (index, width) in widths.iter().enumerate() {
            out.push_str(&format!("control_set {set} {index} {}\n", f(*width)));
        }
    }

    let mut sink = Timelines {
        out: String::new(),
        index: 0,
    };

    // Dense: one pixel at a time, at the shipped row and both timecode widths.
    // 145 px is "0:00 / 0:40"; 232 px is an hour-long track, which is the form
    // that made the original overlap visible.
    for timecode in [BAND_TIMECODES[2], BAND_TIMECODES[3]] {
        for width in 1..=1600 {
            sink.emit(
                0.0,
                0.0,
                width as f32,
                38.0,
                6.0,
                2,
                CONTROL_CLEARS[2],
                timecode,
            );
        }
    }

    // Threshold-adjacent, for every control set x clear variant x margin x
    // timecode. Each base is a width at which one of the module's comparisons
    // flips:
    //   inline_room == natural            -> the row starts shrinking
    //   inline_room == MIN_SCALE*natural  -> the timecode relocates instead
    //   content_width == natural          -> the relocated row starts shrinking
    //   content_width == MIN_SCALE*nat    -> `fits` goes false
    //   content_width == 0                -> nothing is left at all
    // An off-by-one in any of those is the bug class this harness exists for.
    // The index is the compared `in_set` column and also indexes CONTROL_SETS
    // inside `emit`, so it is the loop variable rather than an enumerate() item.
    for (set, table_clear) in CONTROL_CLEARS.iter().enumerate() {
        for clear_variant in 0..2 {
            let clear_width = if clear_variant == 0 {
                *table_clear
            } else {
                0.0
            };
            for margin in BAND_MARGINS {
                let natural = natural_width(set, margin, clear_width);
                for timecode in BAND_TIMECODES {
                    let pad = margin * 2.0;
                    let bases = [
                        0.0,
                        pad,
                        natural + timecode + TIMELINE_BAND_GAP + pad,
                        TIMELINE_BAND_MIN_SCALE * natural + timecode + TIMELINE_BAND_GAP + pad,
                        natural + pad,
                        TIMELINE_BAND_MIN_SCALE * natural + pad,
                        timecode + TIMELINE_BAND_GAP + pad,
                    ];
                    for base in bases {
                        for offset in THRESHOLD_OFFSETS {
                            sink.emit(
                                0.0,
                                0.0,
                                base + offset,
                                38.0,
                                margin,
                                set,
                                clear_width,
                                timecode,
                            );
                        }
                    }
                }
            }
        }
    }

    // The window grid: the band gets the whole window width in the timeline
    // header, and window minus the rail and the inspector in the tightest case
    // `timeline_layout.h:9-21` names. The second derivation goes negative for a
    // narrow window, which is a rejection worth comparing rather than skipping.
    for width in WINDOW_WIDTHS {
        sink.emit(0.0, 0.0, width, 38.0, 6.0, 2, 112.0, 145.0);
        sink.emit(0.0, 0.0, width - 660.0, 38.0, 6.0, 2, 112.0, 232.0);
        sink.emit(0.0, 0.0, width * 0.75 - 320.0, 38.0, 6.0, 1, 112.0, 145.0);
    }

    // Band height is passed straight through to every rect, so it needs a few
    // values rather than a sweep -- including the ones that reject.
    for height in [
        0.0,
        0.0625,
        1.0,
        22.0,
        38.0,
        64.0,
        200.0,
        -1.0,
        1.0e38,
        f32::NAN,
        f32::INFINITY,
    ] {
        sink.emit(0.0, 0.0, 1280.0, height, 6.0, 2, 112.0, 145.0);
        sink.emit(0.0, 0.0, 680.0, height, 6.0, 2, 112.0, 232.0);
    }

    // A band that does not start at the origin, including one far enough out that
    // band_x + band_width loses precision.
    #[rustfmt::skip]
    let origins: [(f32, f32); 10] = [
        (0.0, 0.0), (320.0, 40.0), (-100.0, -50.0),
        (0.5, 0.25), (1.0e30, 1.0e30), (1.0e38, 0.0),
        (f32::NAN, 0.0), (0.0, f32::NAN), (f32::INFINITY, 0.0), (0.0, f32::NEG_INFINITY),
    ];
    for (band_x, band_y) in origins {
        for width in [400.0f32, 620.0, 680.0, 785.0, 960.0, 1280.0, 1920.0] {
            sink.emit(band_x, band_y, width, 38.0, 6.0, 2, 112.0, 145.0);
            sink.emit(band_x, band_y, width, 38.0, 6.0, 2, 112.0, 232.0);
        }
    }

    // Non-finite and negative scalars, one at a time, so each rejection branch is
    // reached on its own rather than masked by an earlier one.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        sink.emit(0.0, 0.0, bad, 38.0, 6.0, 2, 112.0, 145.0);
        sink.emit(0.0, 0.0, 1280.0, 38.0, bad, 2, 112.0, 145.0);
        sink.emit(0.0, 0.0, 1280.0, 38.0, 6.0, 2, bad, 145.0);
        sink.emit(0.0, 0.0, 1280.0, 38.0, 6.0, 2, 112.0, bad);
    }
    // A negative control width, rejected inside the accumulation loop rather than
    // by the argument guard. Built here because the shared table is deliberately
    // all-legal; the C labels these sets 100..103.
    #[rustfmt::skip]
    let poison_widths: [[f32; 6]; 4] = [
        [74.0, -1.0, 90.0, 74.0, 82.0, 86.0],
        [74.0, 74.0, 90.0, 74.0, 82.0, f32::NAN],
        [f32::INFINITY, 74.0, 90.0, 74.0, 82.0, 86.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ];
    for (index, widths) in poison_widths.iter().enumerate() {
        sink.emit_widths(
            0.0,
            0.0,
            1280.0,
            38.0,
            6.0,
            100 + index,
            widths,
            112.0,
            145.0,
        );
    }

    // Zero-length control row, which both sides *can* express: C spells it
    // control_count == 0, Rust an empty slice. Labelled 200 there and here.
    sink.emit_widths(0.0, 0.0, 1280.0, 38.0, 6.0, 200, &[], 112.0, 145.0);

    out.push_str(&sink.out);
}

// ---------------------------------------------------------------------------
// Divergences in mechanism, recorded rather than hidden. The C takes bare
// pointers and a bare enum, so it can refuse inputs this side cannot even
// represent. Both halves print their side of each pair and the driver asserts
// the pair, which keeps the difference visible instead of untested.
// ---------------------------------------------------------------------------

fn dump_divergences(out: &mut String) {
    // A `&mut Self` out-parameter, a `*const f32` array and a bare enum are all
    // things the Rust signatures replaced: the result is a returned value, the
    // widths are a slice, and the mode is a closed enum. None of these five
    // rejections has an input that can be spelled here at all.
    for name in [
        "null_band_out",
        "null_control_widths",
        "null_sidebar_out",
        "null_action_top",
        "null_action_height",
        "out_of_range_mode",
    ] {
        out.push_str(&format!("divergence {name} not_expressible\n"));
    }
}

fn main() {
    let mut out = String::new();
    dump_rect_helpers(&mut out);
    dump_action_rows(&mut out);
    dump_sidebars(&mut out);
    dump_timelines(&mut out);
    dump_divergences(&mut out);
    print!("{out}");
}
