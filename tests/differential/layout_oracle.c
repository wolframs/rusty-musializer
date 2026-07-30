/*
 * Differential harness: dumps the *oracle's* workspace and timeline layout
 * decisions over a dense sweep of window sizes, so the Rust ports can be
 * compared against them numerically instead of by inspection.
 *
 * This compiles `../musializer/src/workspace_layout.c` and
 * `../musializer/src/timeline_layout.c` from the frozen C repository. It reads
 * that tree and writes only into our own `build/`; the oracle is never modified,
 * and its own build directory is never touched.
 *
 * Why these two modules earn a harness rather than more hand-written tests: the
 * ported tests are mostly *property* assertions ("this rect is inside that one"),
 * and a property survives a genuinely wrong formula. A layout drift is also nearly
 * invisible in a screenshot, because everything moves together and still looks
 * plausible. The oracle's own suites have 11 and 18 exact expectations; a
 * differential sweep has hundreds of thousands, and none of them can be
 * mistyped or copied from our own output.
 *
 * The case generator is duplicated in `layout_dump.rs` deliberately, in plain
 * float arithmetic with integer parameters. Two copies of a fixture generator is
 * normally a smell; here it is the entire mechanism, because a shared generator
 * could hide the very difference the test looks for. Every case's *inputs* are
 * printed as compared columns too, so the two copies drifting apart fails the
 * harness rather than silently comparing different sweeps.
 *
 * Record grammar (one line each, whitespace separated; see the SCHEMA table in
 * tools/differential_layout.sh, which must agree):
 *
 *   rect_table   <i> <x> <y> <w> <h>
 *   rect         <i> <j> <a_finite> <a_empty> <b_finite> <b_empty>
 *                    <contains_ab> <contains_ba> <overlaps_ab> <overlaps_ba>
 *                    <isect_ab x y w h> <isect_ba x y w h>
 *   action_row   <mode> <ok> <top> <height>
 *   control_set  <set> <index> <width>
 *   sidebar      <case> <in_width> <in_height> <in_tracks>
 *                    <ok> <tracks x y w h> <scenes x y w h> <mode>
 *   timeline     <case> <in_x> <in_y> <in_width> <in_height> <in_margin>
 *                    <in_set> <in_count> <in_clear> <in_timecode>
 *                    <ok> <scale> <controls_width> <controls x y w h>
 *                    <clear x y w h> <timecode x y w h> <inline> <fits>
 *   divergence   <name> <this side's behaviour>
 *
 * Every float is printed as %.9e — ten significant digits, which round-trips an
 * IEEE single exactly, so the comparison is effectively bit-exact and the
 * tolerance is margin rather than room the port needs.
 *
 * Rejected calls print the *sentinel* the out-parameter was pre-filled with
 * (SENTINEL_F / SENTINEL_MODE), so "returns false and leaves *out untouched" is
 * a compared column rather than a claim. The Rust side prints the same sentinels
 * for its `None`, where the guarantee is structural.
 *
 * Build and run through tools/differential_layout.sh.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <math.h>

#include "workspace_layout.h"
#include "timeline_layout.h"

#define SENTINEL_F (-12345.0f)
#define SENTINEL_MODE 7

static void pf(float value)
{
    printf(" %.9e", (double)value);
}

/* ------------------------------------------------------------------------- *
 * Ui_Rect helpers: is_finite / is_empty / contains / overlaps / intersect.
 *
 * The table is swept as every ordered pair, both ways round, because contains
 * and intersect are asymmetric and the empty/non-finite refusals are exactly
 * where a port drifts. Non-finite entries are deliberate: they are also what
 * exercises the comparator's non-finite path, which AGENTS.md records as a hole
 * a previous harness fell into.
 * ------------------------------------------------------------------------- */

#define RECT_COUNT 29

static Ui_Rect rects[RECT_COUNT];

static void build_rects(void)
{
    size_t i = 0;
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 100.0f, 100.0f};
    rects[i++] = (Ui_Rect){10.0f, 10.0f, 50.0f, 50.0f};
    rects[i++] = (Ui_Rect){-1.0f, 0.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 95.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){50.0f, 50.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){5.0f, 5.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 10.0f, 10.0f, 10.0f};   /* shares an edge */
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 10.0f, 0.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 0.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, -5.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 10.0f, -5.0f};
    rects[i++] = (Ui_Rect){10.0f, 50.0f, 52.0f, 36.0f};  /* a single-mode action row */
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 240.0f, 0.0f};    /* the zero-height panel */
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 240.0f, 188.0f};  /* tracks at a 460 px sidebar */
    rects[i++] = (Ui_Rect){0.0f, 188.0f, 240.0f, 272.0f};/* scenes at the same */
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 320.0f, 168.0f};
    rects[i++] = (Ui_Rect){0.0f, 168.0f, 320.0f, 355.0f};
    rects[i++] = (Ui_Rect){NAN, 10.0f, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, NAN, 10.0f, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, NAN, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 10.0f, NAN};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, INFINITY, 10.0f};
    rects[i++] = (Ui_Rect){0.0f, 0.0f, 10.0f, INFINITY};
    rects[i++] = (Ui_Rect){-INFINITY, 0.0f, 10.0f, 10.0f};
    /* x + width overflows to +inf in float, so the containment arithmetic itself
     * goes non-finite while every field is finite. */
    rects[i++] = (Ui_Rect){1.0e38f, 1.0e38f, 1.0e38f, 1.0e38f};
    rects[i++] = (Ui_Rect){-1.0e38f, 0.0f, 2.0e38f, 10.0f};
    rects[i++] = (Ui_Rect){0.5f, 0.25f, 99.5f, 99.75f};
    rects[i++] = (Ui_Rect){100.0f, 100.0f, 0.0625f, 0.0625f};
    if (i != RECT_COUNT) {
        fprintf(stderr, "rect table size mismatch: %zu vs %d\n", i, RECT_COUNT);
    }
}

static void dump_rect_helpers(void)
{
    for (size_t i = 0; i < RECT_COUNT; ++i) {
        printf("rect_table %zu", i);
        pf(rects[i].x);
        pf(rects[i].y);
        pf(rects[i].width);
        pf(rects[i].height);
        printf("\n");
    }
    for (size_t i = 0; i < RECT_COUNT; ++i) {
        for (size_t j = 0; j < RECT_COUNT; ++j) {
            Ui_Rect a = rects[i];
            Ui_Rect b = rects[j];
            Ui_Rect ab = ui_rect_intersect(a, b);
            Ui_Rect ba = ui_rect_intersect(b, a);
            printf("rect %zu %zu %d %d %d %d %d %d %d %d",
                   i, j,
                   ui_rect_is_finite(a) ? 1 : 0, ui_rect_is_empty(a) ? 1 : 0,
                   ui_rect_is_finite(b) ? 1 : 0, ui_rect_is_empty(b) ? 1 : 0,
                   ui_rect_contains(a, b) ? 1 : 0, ui_rect_contains(b, a) ? 1 : 0,
                   ui_rect_overlaps(a, b) ? 1 : 0, ui_rect_overlaps(b, a) ? 1 : 0);
            pf(ab.x); pf(ab.y); pf(ab.width); pf(ab.height);
            pf(ba.x); pf(ba.y); pf(ba.width); pf(ba.height);
            printf("\n");
        }
    }
}

/* ------------------------------------------------------------------------- *
 * workspace_tracks_action_row. Three modes, and the sentinel proves the hidden
 * mode leaves both out-parameters untouched rather than zeroing them.
 * ------------------------------------------------------------------------- */

static void dump_action_rows(void)
{
    static const Tracks_Panel_Mode modes[] = {
        TRACKS_PANEL_HIDDEN, TRACKS_PANEL_SINGLE, TRACKS_PANEL_STACKED,
    };
    for (size_t i = 0; i < sizeof modes/sizeof modes[0]; ++i) {
        float top = SENTINEL_F;
        float height = SENTINEL_F;
        bool ok = workspace_tracks_action_row(modes[i], &top, &height);
        printf("action_row %d %d", (int)modes[i], ok ? 1 : 0);
        pf(top);
        pf(height);
        printf("\n");
    }
}

/* ------------------------------------------------------------------------- *
 * workspace_sidebar_layout.
 * ------------------------------------------------------------------------- */

static size_t sidebar_case_index = 0;

static void emit_sidebar(float width, float height, size_t track_count)
{
    Workspace_Sidebar out;
    out.tracks = (Ui_Rect){SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F};
    out.scenes = (Ui_Rect){SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F};
    out.tracks_mode = (Tracks_Panel_Mode)SENTINEL_MODE;

    bool ok = workspace_sidebar_layout(width, height, track_count, &out);

    printf("sidebar %zu", sidebar_case_index++);
    pf(width);
    pf(height);
    printf(" %llu %d", (unsigned long long)track_count, ok ? 1 : 0);
    pf(out.tracks.x); pf(out.tracks.y); pf(out.tracks.width); pf(out.tracks.height);
    pf(out.scenes.x); pf(out.scenes.y); pf(out.scenes.width); pf(out.scenes.height);
    printf(" %d\n", (int)out.tracks_mode);
}

/* Duplicated, on purpose, from layout_dump.rs. */
static const float sidebar_dense_widths[] = {168.0f, 320.0f};
static const size_t sidebar_dense_counts[] = {0, 1, 3, 12};
static const float sidebar_grid_widths[] = {
    1.0f, 2.0f, 96.0f, 168.0f, 200.0f, 240.0f, 280.0f, 320.0f, 400.0f, 640.0f, 3000.0f,
};
static const size_t sidebar_grid_counts[] = {0, 1, 2, 3, 4, 5, 8, 12, 64, 512, 4096};
/* Sixteenths, so a threshold-adjacent probe is exactly representable and the two
 * generators cannot disagree about what "one pixel below" means. */
static const float threshold_offsets[] = {-1.0f, -0.0625f, 0.0f, 0.0625f, 1.0f};

/* The window grid: the sizes a user can actually produce, plus the degenerate
 * and absurd ends. 960x640 is the documented minimum and 1280x720 the default. */
static const float window_widths[] = {
    0.0f, 1.0f, 200.0f, 640.0f, 960.0f, 1024.0f, 1280.0f, 1366.0f,
    1600.0f, 1920.0f, 2560.0f, 3840.0f, 7680.0f,
};
static const float window_heights[] = {
    0.0f, 1.0f, 300.0f, 480.0f, 600.0f, 640.0f, 720.0f, 768.0f,
    800.0f, 900.0f, 1080.0f, 1440.0f, 2160.0f, 4320.0f,
};

/* The rail width the shell hands the sidebar, and the sidebar height left once
 * the default 180 px timeline is reserved. Plain arithmetic, duplicated. */
static float rail_width(float window_width)
{
    float rail = window_width*0.25f;
    if (rail < 240.0f) rail = 240.0f;
    if (rail > 320.0f) rail = 320.0f;
    return rail;
}

static void dump_sidebars(void)
{
    /* Dense: one pixel at a time through every threshold, at the rail width the
     * shell produces with no inspector. */
    for (size_t c = 0; c < sizeof sidebar_dense_counts/sizeof sidebar_dense_counts[0]; ++c) {
        for (int h = 1; h <= 1400; ++h) {
            emit_sidebar(240.0f, (float)h, sidebar_dense_counts[c]);
        }
    }
    /* Dense, coarser, at the two other rail widths the shell can produce. */
    for (size_t w = 0; w < sizeof sidebar_dense_widths/sizeof sidebar_dense_widths[0]; ++w) {
        for (size_t c = 0; c < sizeof sidebar_dense_counts/sizeof sidebar_dense_counts[0]; ++c) {
            for (int h = 1; h <= 1400; h += 7) {
                emit_sidebar(sidebar_dense_widths[w], (float)h, sidebar_dense_counts[c]);
            }
        }
    }
    /* Threshold-adjacent: the whole point. Every boundary the module compares
     * against, probed a sixteenth and a pixel either side. */
    for (size_t w = 0; w < sizeof sidebar_grid_widths/sizeof sidebar_grid_widths[0]; ++w) {
        for (size_t c = 0; c < sizeof sidebar_grid_counts/sizeof sidebar_grid_counts[0]; ++c) {
            float width = sidebar_grid_widths[w];
            size_t count = sidebar_grid_counts[c];
            float wanted = WORKSPACE_TRACKS_STACKED_HEADER +
                           (float)count*width*WORKSPACE_TRACKS_ITEM_RATIO;
            if (!isfinite(wanted) || wanted < WORKSPACE_TRACKS_STACKED_MINIMUM) {
                wanted = WORKSPACE_TRACKS_STACKED_MINIMUM;
            }
            if (wanted > WORKSPACE_TRACKS_MAXIMUM) wanted = WORKSPACE_TRACKS_MAXIMUM;
            const float bases[] = {
                wanted,
                wanted + WORKSPACE_SCENES_MINIMUM,
                wanted + WORKSPACE_SCENES_MAXIMUM,
                wanted + WORKSPACE_TRACKS_SINGLE_MINIMUM,
                wanted + WORKSPACE_TRACKS_STACKED_MINIMUM,
                WORKSPACE_SCENES_MINIMUM,
                WORKSPACE_SCENES_MAXIMUM,
                WORKSPACE_TRACKS_SINGLE_MINIMUM,
                WORKSPACE_TRACKS_STACKED_MINIMUM,
            };
            for (size_t b = 0; b < sizeof bases/sizeof bases[0]; ++b) {
                for (size_t o = 0; o < sizeof threshold_offsets/sizeof threshold_offsets[0]; ++o) {
                    emit_sidebar(width, bases[b] + threshold_offsets[o], count);
                }
            }
        }
    }
    /* The window grid, twice: once treating the window as the sidebar directly
     * (extremely wide and extremely tall aspect ratios, and the 0 cases), once
     * through the rail/timeline derivation the shell actually performs. */
    for (size_t w = 0; w < sizeof window_widths/sizeof window_widths[0]; ++w) {
        for (size_t h = 0; h < sizeof window_heights/sizeof window_heights[0]; ++h) {
            for (size_t c = 0; c < 3; ++c) {
                static const size_t counts[] = {0, 1, 3};
                emit_sidebar(window_widths[w], window_heights[h], counts[c]);
                emit_sidebar(rail_width(window_widths[w]),
                             window_heights[h] - 180.0f, counts[c]);
            }
        }
    }
    /* Extremes and non-finite input. The huge counts are what reach the oracle's
     * "an overflowed ask goes to the *minimum*" branch, which the port kept. */
    {
        struct { float width; float height; size_t count; } cases[] = {
            {240.0f, 0.0f, 1},
            {240.0f, -10.0f, 1},
            {0.0f, 600.0f, 1},
            {-1.0f, 600.0f, 1},
            {1.0e-3f, 1.0e-3f, 0},
            {0.0625f, 640.0f, 0},
            {240.0f, 1.0e-3f, 1},
            {1.0e38f, 600.0f, 1},
            {3.0e38f, 600.0f, 1},
            {240.0f, 1.0e38f, 1},
            {240.0f, 3.0e38f, 1},
            {240.0f, 600.0f, SIZE_MAX},
            {1.0e38f, 600.0f, SIZE_MAX},
            {1.0e38f, 1.0e38f, SIZE_MAX},
            {3.0e38f, 3.0e38f, SIZE_MAX},
            {240.0f, 600.0f, (size_t)1 << 40},
            {1.0e20f, 600.0f, (size_t)1 << 40},
            {NAN, 600.0f, 1},
            {240.0f, NAN, 1},
            {NAN, NAN, 1},
            {INFINITY, 600.0f, 1},
            {240.0f, INFINITY, 1},
            {-INFINITY, 600.0f, 1},
            {240.0f, -INFINITY, 1},
        };
        for (size_t i = 0; i < sizeof cases/sizeof cases[0]; ++i) {
            emit_sidebar(cases[i].width, cases[i].height, cases[i].count);
        }
    }
}

/* ------------------------------------------------------------------------- *
 * timeline_band_layout.
 *
 * The parameterised variants matter here: the control row has a different
 * membership in the transport bar than in the event panel, and the trailing
 * clear button is present in one and zero-width in the other. Those
 * combinations are part of the contract, so the sweep drives all of them.
 * ------------------------------------------------------------------------- */

#define CONTROL_SET_COUNT 9
#define CONTROL_MAX 9

static const size_t control_lengths[CONTROL_SET_COUNT] = {1, 3, 6, 8, 3, 2, 4, 5, 9};
static const float control_sets[CONTROL_SET_COUNT][CONTROL_MAX] = {
    {112.0f},
    /* The event panel's marker row (`panels/events.rs` geometry). */
    {74.0f, 82.0f, 86.0f},
    /* The shipped panel/event row the ported tests use. */
    {74.0f, 74.0f, 90.0f, 74.0f, 82.0f, 86.0f},
    /* Exactly TIMELINE_BAND_CONTROL_CAPACITY. */
    {40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f},
    /* Zero widths: natural comes only from the margins and the clear button. */
    {0.0f, 0.0f, 0.0f},
    {220.0f, 96.0f},
    {33.5f, 47.25f, 61.0625f, 12.5f},
    /* The transport bar's six buttons, rounded to what the measurer yields. */
    {64.0f, 64.0f, 64.0f, 64.0f, 64.0f},
    /* One past capacity: both sides can express this rejection. */
    {40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f, 40.0f},
};
static const float control_clears[CONTROL_SET_COUNT] = {
    112.0f, 112.0f, 112.0f, 0.0f, 112.0f, 0.0f, 18.5f, 0.0f, 112.0f,
};

static const float band_margins[] = {0.0f, 6.0f, 12.0f};
static const float band_timecodes[] = {0.0f, 90.0f, 145.0f, 232.0f, 400.0f};

static size_t timeline_case_index = 0;

static void emit_timeline_widths(float band_x, float band_y, float band_width,
                                 float band_height, float margin, size_t set_label,
                                 const float *control_widths, size_t count,
                                 float clear_width, float timecode_width)
{
    Timeline_Band out;
    out.scale = SENTINEL_F;
    out.controls_width = SENTINEL_F;
    out.controls = (Ui_Rect){SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F};
    out.clear = (Ui_Rect){SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F};
    out.timecode = (Ui_Rect){SENTINEL_F, SENTINEL_F, SENTINEL_F, SENTINEL_F};
    out.timecode_inline = false;
    out.fits = false;

    bool ok = timeline_band_layout(band_x, band_y, band_width, band_height, margin,
                                   control_widths, count,
                                   clear_width, timecode_width, &out);

    printf("timeline %zu", timeline_case_index++);
    pf(band_x);
    pf(band_y);
    pf(band_width);
    pf(band_height);
    pf(margin);
    printf(" %zu %zu", set_label, count);
    pf(clear_width);
    pf(timecode_width);
    printf(" %d", ok ? 1 : 0);
    pf(out.scale);
    pf(out.controls_width);
    pf(out.controls.x); pf(out.controls.y); pf(out.controls.width); pf(out.controls.height);
    pf(out.clear.x); pf(out.clear.y); pf(out.clear.width); pf(out.clear.height);
    pf(out.timecode.x); pf(out.timecode.y); pf(out.timecode.width); pf(out.timecode.height);
    printf(" %d %d\n", out.timecode_inline ? 1 : 0, out.fits ? 1 : 0);
}

static void emit_timeline(float band_x, float band_y, float band_width,
                          float band_height, float margin, size_t set,
                          float clear_width, float timecode_width)
{
    emit_timeline_widths(band_x, band_y, band_width, band_height, margin, set,
                         control_sets[set], control_lengths[set],
                         clear_width, timecode_width);
}

/* The row's unscaled extent, recomputed here rather than read back out of the
 * result, so the threshold probes do not depend on the thing under test. */
static float natural_width(size_t set, float margin, float clear_width)
{
    float natural = clear_width;
    for (size_t i = 0; i < control_lengths[set]; ++i) {
        natural += control_sets[set][i] + margin;
    }
    return natural;
}

static void dump_timelines(void)
{
    for (size_t set = 0; set < CONTROL_SET_COUNT; ++set) {
        for (size_t i = 0; i < control_lengths[set]; ++i) {
            printf("control_set %zu %zu", set, i);
            pf(control_sets[set][i]);
            printf("\n");
        }
    }

    /* Dense: one pixel at a time, at the shipped row and both timecode widths.
     * 145 px is "0:00 / 0:40"; 232 px is an hour-long track, which is the form
     * that made the original overlap visible. */
    for (size_t t = 2; t <= 3; ++t) {
        for (int width = 1; width <= 1600; ++width) {
            emit_timeline(0.0f, 0.0f, (float)width, 38.0f, 6.0f, 2,
                          control_clears[2], band_timecodes[t]);
        }
    }

    /* Threshold-adjacent, for every control set x margin x timecode. Each base is
     * a width at which one of the module's comparisons flips:
     *   inline_room == natural            -> the row starts shrinking
     *   inline_room == MIN_SCALE*natural  -> the timecode relocates instead
     *   content_width == natural          -> the relocated row starts shrinking
     *   content_width == MIN_SCALE*nat    -> `fits` goes false
     *   content_width == 0                -> nothing is left at all
     * An off-by-one in any of those is the bug class this harness exists for. */
    for (size_t set = 0; set < CONTROL_SET_COUNT; ++set) {
        for (size_t clear_variant = 0; clear_variant < 2; ++clear_variant) {
            float clear_width = clear_variant == 0 ? control_clears[set] : 0.0f;
            for (size_t m = 0; m < sizeof band_margins/sizeof band_margins[0]; ++m) {
                float margin = band_margins[m];
                float natural = natural_width(set, margin, clear_width);
                for (size_t t = 0; t < sizeof band_timecodes/sizeof band_timecodes[0]; ++t) {
                    float timecode = band_timecodes[t];
                    float pad = margin*2.0f;
                    const float bases[] = {
                        0.0f,
                        pad,
                        natural + timecode + TIMELINE_BAND_GAP + pad,
                        TIMELINE_BAND_MIN_SCALE*natural + timecode + TIMELINE_BAND_GAP + pad,
                        natural + pad,
                        TIMELINE_BAND_MIN_SCALE*natural + pad,
                        timecode + TIMELINE_BAND_GAP + pad,
                    };
                    for (size_t b = 0; b < sizeof bases/sizeof bases[0]; ++b) {
                        for (size_t o = 0;
                             o < sizeof threshold_offsets/sizeof threshold_offsets[0]; ++o) {
                            emit_timeline(0.0f, 0.0f, bases[b] + threshold_offsets[o],
                                          38.0f, margin, set, clear_width, timecode);
                        }
                    }
                }
            }
        }
    }

    /* The window grid: the band gets the whole window width in the timeline
     * header, and window minus the rail and the inspector in the tightest case
     * `timeline_layout.h:9-21` names. The second derivation goes negative for a
     * narrow window, which is a rejection worth comparing rather than skipping. */
    for (size_t w = 0; w < sizeof window_widths/sizeof window_widths[0]; ++w) {
        emit_timeline(0.0f, 0.0f, window_widths[w], 38.0f, 6.0f, 2, 112.0f, 145.0f);
        emit_timeline(0.0f, 0.0f, window_widths[w] - 660.0f, 38.0f, 6.0f, 2, 112.0f, 232.0f);
        emit_timeline(0.0f, 0.0f, window_widths[w]*0.75f - 320.0f, 38.0f, 6.0f, 1,
                      112.0f, 145.0f);
    }

    /* Band height is passed straight through to every rect, so it needs a few
     * values rather than a sweep -- including the ones that reject. */
    {
        static const float heights[] = {
            0.0f, 0.0625f, 1.0f, 22.0f, 38.0f, 64.0f, 200.0f,
            -1.0f, 1.0e38f, NAN, INFINITY,
        };
        for (size_t h = 0; h < sizeof heights/sizeof heights[0]; ++h) {
            emit_timeline(0.0f, 0.0f, 1280.0f, heights[h], 6.0f, 2, 112.0f, 145.0f);
            emit_timeline(0.0f, 0.0f, 680.0f, heights[h], 6.0f, 2, 112.0f, 232.0f);
        }
    }

    /* A band that does not start at the origin, including one far enough out that
     * band_x + band_width loses precision. */
    {
        static const float origins[][2] = {
            {0.0f, 0.0f}, {320.0f, 40.0f}, {-100.0f, -50.0f},
            {0.5f, 0.25f}, {1.0e30f, 1.0e30f}, {1.0e38f, 0.0f},
            {NAN, 0.0f}, {0.0f, NAN}, {INFINITY, 0.0f}, {0.0f, -INFINITY},
        };
        static const float widths[] = {400.0f, 620.0f, 680.0f, 785.0f, 960.0f, 1280.0f, 1920.0f};
        for (size_t o = 0; o < sizeof origins/sizeof origins[0]; ++o) {
            for (size_t w = 0; w < sizeof widths/sizeof widths[0]; ++w) {
                emit_timeline(origins[o][0], origins[o][1], widths[w], 38.0f, 6.0f, 2,
                              112.0f, 145.0f);
                emit_timeline(origins[o][0], origins[o][1], widths[w], 38.0f, 6.0f, 2,
                              112.0f, 232.0f);
            }
        }
    }

    /* Non-finite and negative scalars, one at a time, so each rejection branch is
     * reached on its own rather than masked by an earlier one. */
    {
        static const float poisons[] = {NAN, INFINITY, -INFINITY, -1.0f};
        for (size_t p = 0; p < sizeof poisons/sizeof poisons[0]; ++p) {
            float bad = poisons[p];
            emit_timeline(0.0f, 0.0f, bad, 38.0f, 6.0f, 2, 112.0f, 145.0f);
            emit_timeline(0.0f, 0.0f, 1280.0f, 38.0f, bad, 2, 112.0f, 145.0f);
            emit_timeline(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f, 2, bad, 145.0f);
            emit_timeline(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f, 2, 112.0f, bad);
        }
        /* A negative control width, which is rejected inside the accumulation
         * loop rather than by the argument guard. The sets are built here because
         * the shared table is deliberately all-legal; the labels 100..103 name
         * this table, and nothing indexes it. */
        static const float poison_widths[][6] = {
            {74.0f, -1.0f, 90.0f, 74.0f, 82.0f, 86.0f},
            {74.0f, 74.0f, 90.0f, 74.0f, 82.0f, NAN},
            {INFINITY, 74.0f, 90.0f, 74.0f, 82.0f, 86.0f},
            {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f},
        };
        for (size_t p = 0; p < sizeof poison_widths/sizeof poison_widths[0]; ++p) {
            emit_timeline_widths(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f, 100 + p,
                                 poison_widths[p], 6, 112.0f, 145.0f);
        }
    }

    /* Zero-length control row, which both sides *can* express: C spells it
     * control_count == 0, Rust an empty slice. Labelled 200 on both sides. The
     * pointer is legal here; only the count is zero. */
    emit_timeline_widths(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f, 200,
                         control_sets[2], 0, 112.0f, 145.0f);
}

/* ------------------------------------------------------------------------- *
 * Divergences in mechanism, recorded rather than hidden. The C takes bare
 * pointers and a bare enum, so it can refuse inputs the Rust side cannot even
 * represent. Both halves print their side of each pair and the driver asserts
 * the pair, which keeps the difference visible instead of untested.
 *
 * The zero-length control row is *not* here: C spells it control_count == 0 and
 * Rust spells it an empty slice, so both can express it. dump_timelines emits it
 * as an ordinary timeline record under the synthetic set label 200.
 * ------------------------------------------------------------------------- */
static void dump_divergences(void)
{
    Timeline_Band band;
    float top = SENTINEL_F;

    bool null_band_out = timeline_band_layout(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f,
                                              control_sets[2], 6, 112.0f, 145.0f, NULL);
    bool null_control_widths = timeline_band_layout(0.0f, 0.0f, 1280.0f, 38.0f, 6.0f,
                                                    NULL, 6, 112.0f, 145.0f, &band);
    bool null_sidebar_out = workspace_sidebar_layout(240.0f, 600.0f, 1, NULL);
    bool null_action_top = workspace_tracks_action_row(TRACKS_PANEL_SINGLE, NULL, &top);
    bool null_action_height = workspace_tracks_action_row(TRACKS_PANEL_SINGLE, &top, NULL);
    /* An out-of-range enum value falls off the switch and returns false. Rust's
     * TracksPanelMode has no such value, which is the divergence. */
    bool out_of_range_mode = workspace_tracks_action_row((Tracks_Panel_Mode)99, &top, &top);

    printf("divergence null_band_out %s\n", null_band_out ? "c_accepts" : "c_rejects");
    printf("divergence null_control_widths %s\n",
           null_control_widths ? "c_accepts" : "c_rejects");
    printf("divergence null_sidebar_out %s\n", null_sidebar_out ? "c_accepts" : "c_rejects");
    printf("divergence null_action_top %s\n", null_action_top ? "c_accepts" : "c_rejects");
    printf("divergence null_action_height %s\n",
           null_action_height ? "c_accepts" : "c_rejects");
    printf("divergence out_of_range_mode %s\n",
           out_of_range_mode ? "c_accepts" : "c_rejects");
}

int main(void)
{
    build_rects();
    dump_rect_helpers();
    dump_action_rows();
    dump_sidebars();
    dump_timelines();
    dump_divergences();
    return 0;
}
