/*
 * Differential harness: dumps the *oracle's* ascii_art output for a set of
 * deterministic synthetic pixel buffers, so the Rust port can be compared
 * against it numerically instead of by inspection.
 *
 * This compiles `../musializer/src/ascii_art.c` from the frozen C repository. It
 * reads that tree and writes only into our own `build/`; the oracle is never
 * modified, and its own build directory is never touched.
 *
 * The whole module is pure — RGBA8 pixels in, a grid of glyph cells out — which
 * is exactly the shape AGENTS.md says a number should settle. Six surfaces are
 * dumped:
 *
 *   fit        ascii_art_fit_grid_dimensions over a source x maximum sweep
 *   layout     ascii_art_grid_layout, including its rejection cases
 *   populated  ascii_art_grid_is_populated
 *   convert    ascii_art_convert_rgba8, every field of every cell, plus the
 *              rejection cases and the atomicity of a rejected call
 *   grid       plug.c:860-891's fit-then-convert composition, the --ascii-image
 *              path, against the Rust `Grid::from_rgba8` that replaced it
 *   anim       ascii_art_animated_glyph over cell x position x time x activity
 *              x seed
 *
 * The pixel fixtures are integer-only arithmetic and are duplicated in
 * `crates/musializer-core/examples/ascii_art_dump.rs` deliberately. Two copies of
 * a fixture generator is normally a smell; here it is the entire mechanism,
 * because a shared generator could hide the very difference the test looks for.
 *
 * Build and run through tools/differential_ascii_art.sh.
 */

#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <math.h>
#include <limits.h>

#include "ascii_art.h"

/* 128x72 is the largest source any case uses. */
#define MAX_PIXEL_BYTES (128u*72u*4u)

static uint8_t pixel_buffer[MAX_PIXEL_BYTES];
static AsciiCell cell_buffer[ASCII_GRID_MAX_CELLS + 8];

/* Printed back after a rejected call, so a write on the failure path shows up as
 * a diff rather than passing silently. */
#define FIT_SENTINEL ((size_t)999999u)
#define LAYOUT_SENTINEL (-12345.0f)
#define CELL_SENTINEL ((uint32_t)0x12345678u)

/*
 * ---------------------------------------------------------------------------
 * Fixtures. Integer arithmetic only, so "the two sides saw identical input"
 * needs no float agreement to establish. Duplicated in the Rust dump on purpose.
 * ---------------------------------------------------------------------------
 */
static void fill_fixture(unsigned kind, uint8_t *pixels, size_t width, size_t height)
{
    for (size_t y = 0; y < height; ++y) {
        for (size_t x = 0; x < width; ++x) {
            size_t index = y*width + x;
            unsigned r = 0, g = 0, b = 0, a = 255;
            switch (kind) {
            case 0: /* smooth two-axis gradient with a quadratic blue channel */
                r = (unsigned)((x*7u + y*3u) & 0xffu);
                g = (unsigned)((x*13u + y*29u) & 0xffu);
                b = (unsigned)(((x*x + y*y)*5u) & 0xffu);
                a = 255u;
                break;
            case 1: { /* hard quadrant edges with a diagonal overlay */
                unsigned value = 0u;
                if (x >= width/2u) value += 160u;
                if (y >= height/2u) value += 80u;
                if ((x + y) % 5u == 0u) value = 255u - value;
                r = g = b = value;
                a = 255u;
            } break;
            case 2: /* varying alpha, including fully transparent pixels */
                r = (unsigned)((x*31u) & 0xffu);
                g = (unsigned)((y*47u) & 0xffu);
                b = (unsigned)(((x + y)*11u) & 0xffu);
                a = ((x*37u + y*53u) % 6u == 0u)
                    ? 0u : (unsigned)((x*17u + y*23u) & 0xffu);
                break;
            case 3: /* byte noise across all four channels, alpha included */
                r = (unsigned)(((index*4u + 0u)*73u + 19u) & 0xffu);
                g = (unsigned)(((index*4u + 1u)*73u + 19u) & 0xffu);
                b = (unsigned)(((index*4u + 2u)*73u + 19u) & 0xffu);
                a = (unsigned)(((index*4u + 3u)*73u + 19u) & 0xffu);
                break;
            case 4: /* flat dark grey: the sparse-ink lift with no edges at all */
                r = g = b = 32u;
                a = 255u;
                break;
            case 5: /* checkerboard: maximum gradient everywhere, no coherence */
                r = g = b = ((x + y) & 1u) ? 255u : 0u;
                a = 255u;
                break;
            case 6: /* a 24-wide luminance band: the no-op contrast stretch */
                r = g = b = (unsigned)(40u + (x*3u + y*5u) % 24u);
                a = 255u;
                break;
            case 7: /* fully transparent: the alpha == 0 colour branch */
                r = (unsigned)((x*9u) & 0xffu);
                g = (unsigned)((y*11u) & 0xffu);
                b = (unsigned)(((x ^ y)*13u) & 0xffu);
                a = 0u;
                break;
            case 8: /* one bright pixel: the percentile edges of the histogram */
                r = g = b = (x == width/2u && y == height/2u) ? 255u : 0u;
                a = 255u;
                break;
            default:
                r = g = b = 0u;
                a = 255u;
                break;
            }
            pixels[index*4u + 0u] = (uint8_t)r;
            pixels[index*4u + 1u] = (uint8_t)g;
            pixels[index*4u + 2u] = (uint8_t)b;
            pixels[index*4u + 3u] = (uint8_t)a;
        }
    }
}

struct Size {
    size_t width;
    size_t height;
};

/*
 * ---------------------------------------------------------------------------
 * fit
 * ---------------------------------------------------------------------------
 */
static const struct Size fit_sources[] = {
    {0, 0}, {0, 100}, {100, 0},
    {1, 1}, {1, 2}, {2, 1}, {3, 1}, {1, 3},
    {16, 9}, {9, 16}, {40, 20}, {20, 40},
    {96, 54}, {54, 96},
    {1408, 768}, {768, 1408},
    {1920, 1080}, {1080, 1920},
    {3840, 2160}, {2160, 3840},
    {10000, 1}, {1, 10000},
    {100000, 3}, {3, 100000},
    {65536, 65535}, {65535, 65536},
    {12345, 6789}, {6789, 12345},
    {SIZE_MAX, 1}, {1, SIZE_MAX},
    {SIZE_MAX, SIZE_MAX}, {SIZE_MAX, 2}, {2, SIZE_MAX},
    {SIZE_MAX/2u, 3}, {3, SIZE_MAX/2u},
};

static const struct Size fit_maxima[] = {
    {0, 0}, {0, 54}, {96, 0},
    {1, 1}, {2, 1}, {1, 2},
    {96, 54}, {320, 200}, {40, 20},
    {SIZE_MAX, SIZE_MAX}, {SIZE_MAX, 54}, {96, SIZE_MAX},
};

static void dump_fit(void)
{
    size_t sources = sizeof(fit_sources)/sizeof(fit_sources[0]);
    size_t maxima = sizeof(fit_maxima)/sizeof(fit_maxima[0]);
    for (size_t s = 0; s < sources; ++s) {
        for (size_t m = 0; m < maxima; ++m) {
            size_t columns = FIT_SENTINEL;
            size_t rows = FIT_SENTINEL;
            bool ok = ascii_art_fit_grid_dimensions(
                fit_sources[s].width, fit_sources[s].height,
                fit_maxima[m].width, fit_maxima[m].height,
                &columns, &rows);
            printf("fit %zu %zu %zu %zu %d %zu %zu\n",
                   fit_sources[s].width, fit_sources[s].height,
                   fit_maxima[m].width, fit_maxima[m].height,
                   ok ? 1 : 0, columns, rows);
        }
    }
}

/*
 * ---------------------------------------------------------------------------
 * layout
 * ---------------------------------------------------------------------------
 */
struct LayoutCase {
    float area_width;
    float area_height;
    size_t columns;
    size_t rows;
    float cell_aspect;
};

static void dump_layout(void)
{
    const struct LayoutCase cases[] = {
        {640.0f, 360.0f, 96, 52, 1.0f},
        {1280.0f, 720.0f, 96, 52, 1.0f},
        {640.0f, 360.0f, 96, 26, 0.5f},
        {1920.0f, 1080.0f, 96, 54, 1.0f},
        {100.0f, 100.0f, 1, 1, 1.0f},
        {3.5f, 7.25f, 7, 3, 2.0f},
        {1000.0f, 10.0f, 96, 54, 1.0f},
        {10.0f, 1000.0f, 96, 54, 1.0f},
        {1e30f, 1e30f, 96, 54, 1.0f},
        {1e-30f, 1e-30f, 96, 54, 1.0f},
        {640.0f, 360.0f, (size_t)INT_MAX, 52, 1.0f},
        /* rejections */
        {100.0f, 100.0f, 0, 10, 1.0f},
        {100.0f, 100.0f, 10, 0, 1.0f},
        {NAN, 360.0f, 96, 52, 1.0f},
        {640.0f, NAN, 96, 52, 1.0f},
        {640.0f, 360.0f, 96, 52, NAN},
        {INFINITY, 360.0f, 96, 52, 1.0f},
        {0.0f, 360.0f, 96, 52, 1.0f},
        {640.0f, -1.0f, 96, 52, 1.0f},
        {640.0f, 360.0f, 96, 52, 0.0f},
        {640.0f, 360.0f, 96, 52, -1.0f},
        {640.0f, 360.0f, (size_t)INT_MAX + 1u, 52, 1.0f},
    };
    size_t count = sizeof(cases)/sizeof(cases[0]);
    for (size_t i = 0; i < count; ++i) {
        AsciiGridLayout layout = {
            LAYOUT_SENTINEL, LAYOUT_SENTINEL, LAYOUT_SENTINEL,
            LAYOUT_SENTINEL, LAYOUT_SENTINEL, LAYOUT_SENTINEL,
        };
        bool ok = ascii_art_grid_layout(cases[i].area_width, cases[i].area_height,
                                        cases[i].columns, cases[i].rows,
                                        cases[i].cell_aspect, &layout);
        printf("layout %zu %d %.9g %.9g %.9g %.9g %.9g %.9g\n",
               i, ok ? 1 : 0,
               (double)layout.cell_width, (double)layout.cell_height,
               (double)layout.field_width, (double)layout.field_height,
               (double)layout.offset_x, (double)layout.offset_y);
    }
}

/*
 * ---------------------------------------------------------------------------
 * populated
 * ---------------------------------------------------------------------------
 */
static void dump_populated(void)
{
    const struct Size cases[] = {
        {0, 0}, {0, 1}, {1, 0}, {1, 1}, {96, 54},
        {SIZE_MAX, 1}, {1, SIZE_MAX}, {SIZE_MAX, SIZE_MAX}, {0, SIZE_MAX},
    };
    size_t count = sizeof(cases)/sizeof(cases[0]);
    for (size_t i = 0; i < count; ++i) {
        printf("populated %zu %zu %d\n", cases[i].width, cases[i].height,
               ascii_art_grid_is_populated(cases[i].width, cases[i].height) ? 1 : 0);
    }
}

/*
 * ---------------------------------------------------------------------------
 * convert
 * ---------------------------------------------------------------------------
 */
struct ConvertCase {
    unsigned kind;
    size_t source_width;
    size_t source_height;
    size_t grid_width;
    size_t grid_height;
};

static const struct ConvertCase convert_cases[] = {
    {0, 1, 1, 1, 1},
    {0, 2, 2, 1, 1},
    {0, 2, 2, 2, 2},
    {0, 3, 3, 2, 2},      /* Bresenham with a remainder in both axes */
    {0, 5, 5, 5, 5},
    {0, 5, 5, 2, 2},
    {0, 7, 5, 3, 2},
    {0, 37, 23, 11, 7},
    {0, 64, 1, 4, 1},
    {0, 1, 64, 1, 4},
    {0, 96, 54, 1, 1},
    {0, 128, 72, 96, 54},
    {1, 5, 5, 1, 1},
    {1, 7, 7, 1, 1},
    {1, 9, 9, 1, 1},
    {1, 16, 9, 16, 9},
    {1, 128, 72, 32, 18},
    {1, 128, 72, 96, 54},
    {2, 8, 6, 4, 3},
    {2, 37, 23, 12, 8},
    {2, 96, 54, 96, 54},
    {3, 8, 6, 4, 3},
    {3, 64, 64, 21, 21},
    {3, 100, 60, 96, 54},
    {4, 1, 1, 1, 1},
    {4, 16, 16, 4, 4},
    {5, 9, 9, 3, 3},
    {5, 64, 36, 64, 36},
    {5, 65, 37, 32, 18},
    {6, 64, 1, 4, 1},
    {6, 40, 30, 20, 15},
    {6, 40, 30, 7, 5},
    {7, 4, 4, 2, 2},
    {7, 33, 17, 11, 5},
    {8, 11, 11, 11, 11},
    {8, 11, 11, 1, 1},
};

static void seed_cells(size_t count)
{
    for (size_t j = 0; j < count; ++j) {
        memset(&cell_buffer[j], 0, sizeof(cell_buffer[j]));
        cell_buffer[j].glyph = CELL_SENTINEL;
    }
}

static void print_cell(const char *record, size_t case_index, size_t cell_index,
                       const AsciiCell *cell)
{
    printf("%s %zu %zu %u %u %u %u %u %.9g %.9g %d\n",
           record, case_index, cell_index,
           (unsigned)cell->glyph,
           (unsigned)cell->foreground.r,
           (unsigned)cell->foreground.g,
           (unsigned)cell->foreground.b,
           (unsigned)cell->foreground.a,
           (double)cell->luminance,
           (double)cell->edge_strength,
           (int)cell->edge_orientation);
}

static void dump_convert(void)
{
    size_t count = sizeof(convert_cases)/sizeof(convert_cases[0]);
    for (size_t i = 0; i < count; ++i) {
        const struct ConvertCase *c = &convert_cases[i];
        size_t cells = c->grid_width*c->grid_height;
        fill_fixture(c->kind, pixel_buffer, c->source_width, c->source_height);
        seed_cells(ASCII_GRID_MAX_CELLS + 8);
        bool ok = ascii_art_convert_rgba8(pixel_buffer,
                                          c->source_width, c->source_height,
                                          c->grid_width, c->grid_height,
                                          cell_buffer, cells);
        printf("convert %zu %u %zu %zu %zu %zu %d %zu\n", i, c->kind,
               c->source_width, c->source_height,
               c->grid_width, c->grid_height, ok ? 1 : 0, cells);
        if (!ok) continue;
        for (size_t j = 0; j < cells; ++j) print_cell("cell", i, j, &cell_buffer[j]);
    }
}

/*
 * ---------------------------------------------------------------------------
 * convert rejections. Half the contract: the C returns false for a zero
 * dimension, a grid larger than the source, an undersized destination, and both
 * accumulator-overflow guards. The two overflow cases pass dimensions no
 * allocation could back, which is safe precisely because dimensions_valid
 * rejects them before a single pixel is read.
 * ---------------------------------------------------------------------------
 */
struct RejectCase {
    const char *label;
    size_t source_width;
    size_t source_height;
    size_t grid_width;
    size_t grid_height;
    size_t output_count;
};

static const struct RejectCase reject_cases[] = {
    {"zero_source_width", 0, 1, 1, 1, 2},
    {"zero_source_height", 1, 0, 1, 1, 2},
    {"zero_grid_width", 1, 1, 0, 1, 2},
    {"zero_grid_height", 1, 1, 1, 0, 2},
    {"grid_wider_than_source", 1, 1, 2, 1, 2},
    {"grid_taller_than_source", 1, 1, 1, 2, 2},
    {"output_too_small", 2, 2, 2, 2, 3},
    {"output_empty", 1, 1, 1, 1, 0},
    {"source_dimensions_overflow", SIZE_MAX, 2, 1, 1, 2},
    {"source_count_accumulator_overflow", 16777216, 16777216, 1, 1, 2},
};

static void dump_rejections(void)
{
    size_t count = sizeof(reject_cases)/sizeof(reject_cases[0]);
    fill_fixture(0, pixel_buffer, 2, 2);
    for (size_t i = 0; i < count; ++i) {
        const struct RejectCase *c = &reject_cases[i];
        seed_cells(4);
        bool ok = ascii_art_convert_rgba8(pixel_buffer,
                                          c->source_width, c->source_height,
                                          c->grid_width, c->grid_height,
                                          cell_buffer, c->output_count);
        /* The two guards prove atomicity: a rejected call must leave the
         * caller's buffer exactly as it found it. */
        printf("reject %s %d %u %u\n", c->label, ok ? 1 : 0,
               (unsigned)cell_buffer[0].glyph, (unsigned)cell_buffer[1].glyph);
    }
}

/*
 * ---------------------------------------------------------------------------
 * Divergences in mechanism, recorded rather than hidden. The Rust side takes
 * slices, so it cannot express a null pointer and *can* detect a pixel buffer
 * shorter than the dimensions claim; the C takes bare pointers, so it is the
 * other way round. Both halves print their side of each pair and the driver
 * asserts the pair, which keeps the difference visible instead of untested.
 * ---------------------------------------------------------------------------
 */
static void dump_divergences(void)
{
    fill_fixture(0, pixel_buffer, 2, 2);
    seed_cells(4);
    bool null_pixels = ascii_art_convert_rgba8(NULL, 2, 2, 2, 2, cell_buffer, 4);
    bool null_output = ascii_art_convert_rgba8(pixel_buffer, 2, 2, 2, 2, NULL, 4);
    bool null_cell = ascii_art_animated_glyph(NULL, 0, 0, 0.0, 0.0f, 0) == 0u;
    printf("divergence null_pixels %s\n", null_pixels ? "c_accepts" : "c_rejects");
    printf("divergence null_output %s\n", null_output ? "c_accepts" : "c_rejects");
    printf("divergence null_cell %s\n", null_cell ? "c_rejects" : "c_accepts");
    /* Deliberately not called with a short buffer: ascii_art_convert_rgba8 has
     * no length parameter, so doing so would read past the end. That inability
     * is the divergence. */
    printf("divergence truncated_pixels c_cannot_detect\n");
}

/*
 * ---------------------------------------------------------------------------
 * grid: plug.c:860-891's fit-then-convert composition, which is what
 * `--ascii-image` runs and what the Rust `Grid::from_rgba8` replaced.
 * ---------------------------------------------------------------------------
 */
struct GridCase {
    unsigned kind;
    size_t source_width;
    size_t source_height;
};

static const struct GridCase grid_cases[] = {
    {0, 128, 72},
    {1, 100, 60},
    {2, 40, 120},
    {3, 8, 6},
    {5, 65, 37},
    {8, 11, 11},
    {4, 3840, 1},   /* a 4K-wide strip must still fit the fixed 96x54 buffer */
};

static void dump_grids(void)
{
    size_t count = sizeof(grid_cases)/sizeof(grid_cases[0]);
    for (size_t i = 0; i < count; ++i) {
        const struct GridCase *c = &grid_cases[i];
        size_t bytes = c->source_width*c->source_height*4u;
        if (bytes > MAX_PIXEL_BYTES) {
            fprintf(stderr, "grid case %zu needs %zu bytes\n", i, bytes);
            return;
        }
        fill_fixture(c->kind, pixel_buffer, c->source_width, c->source_height);
        size_t columns = FIT_SENTINEL;
        size_t rows = FIT_SENTINEL;
        bool ok = ascii_art_fit_grid_dimensions(
            c->source_width, c->source_height,
            ASCII_GRID_MAX_COLUMNS, ASCII_GRID_MAX_ROWS, &columns, &rows);
        if (ok) {
            seed_cells(ASCII_GRID_MAX_CELLS + 8);
            ok = ascii_art_convert_rgba8(pixel_buffer,
                                         c->source_width, c->source_height,
                                         columns, rows,
                                         cell_buffer, ASCII_GRID_MAX_CELLS);
        }
        if (!ok) {
            columns = 0;
            rows = 0;
        }
        printf("grid %zu %u %zu %zu %d %zu %zu\n", i, c->kind,
               c->source_width, c->source_height, ok ? 1 : 0, columns, rows);
        if (!ok) continue;
        for (size_t j = 0; j < columns*rows; ++j) {
            print_cell("gridcell", i, j, &cell_buffer[j]);
        }
    }
}

/*
 * ---------------------------------------------------------------------------
 * anim. Every axis is an index into a table duplicated on the Rust side, so a
 * NaN or an infinity can be driven without having to print one.
 * ---------------------------------------------------------------------------
 */
static void dump_anim(void)
{
    AsciiCell cells[11];
    memset(cells, 0, sizeof(cells));
    /* Field order: glyph, foreground, luminance, edge_strength, orientation. */
    cells[0] = (AsciiCell){'L', {0, 0, 0, 0}, 0.58f, 0.0f, ASCII_EDGE_NONE};
    cells[1] = (AsciiCell){'G', {0, 0, 0, 0}, 0.72f, 0.0f, ASCII_EDGE_NONE};
    cells[2] = (AsciiCell){' ', {0, 0, 0, 0}, 0.0f, 0.0f, ASCII_EDGE_NONE};
    cells[3] = (AsciiCell){' ', {0, 0, 0, 0}, 0.5f, 0.0f, ASCII_EDGE_NONE};
    cells[4] = (AsciiCell){'/', {0, 0, 0, 0}, 0.58f, 0.4f, ASCII_EDGE_DIAGONAL_FORWARD};
    /* An out-of-range codepoint on an edge cell: the C substitutes U+FFFD. */
    cells[5] = (AsciiCell){0x110001u, {0, 0, 0, 0}, 0.5f, 0.4f, ASCII_EDGE_HORIZONTAL};
    cells[6] = (AsciiCell){'@', {0, 0, 0, 0}, 1.0f, 0.0f, ASCII_EDGE_NONE};
    cells[7] = (AsciiCell){'.', {0, 0, 0, 0}, 0.02f, 0.0f, ASCII_EDGE_NONE};
    cells[8] = (AsciiCell){'C', {0, 0, 0, 0}, NAN, 0.0f, ASCII_EDGE_NONE};
    cells[9] = (AsciiCell){'#', {0, 0, 0, 0}, 1.5f, 0.0f, ASCII_EDGE_NONE};
    cells[10] = (AsciiCell){'8', {0, 0, 0, 0}, -0.3f, 0.0f, ASCII_EDGE_NONE};

    const size_t rows[] = {0, 7, 19, 53};
    const size_t columns[] = {0, 11, 31, 95};
    const double times[] = {
        0.0, 0.0625, 0.125, 0.25, 0.5, 1.0, 1.5, 2.75,
        7.125, 47.125, 123.5, 4096.0, 4097.25, -3.5, NAN, INFINITY,
    };
    const float activities[] = {0.0f, 0.37f, 1.0f, 2.5f, NAN};
    const uint64_t seeds[] = {
        0u, 0x1234u, 0xfeedbeefu, UINT64_C(0xffffffffffffffff),
    };

    size_t cell_count = sizeof(cells)/sizeof(cells[0]);
    size_t position_count = sizeof(rows)/sizeof(rows[0]);
    size_t time_count = sizeof(times)/sizeof(times[0]);
    size_t activity_count = sizeof(activities)/sizeof(activities[0]);
    size_t seed_count = sizeof(seeds)/sizeof(seeds[0]);

    for (size_t c = 0; c < cell_count; ++c) {
        for (size_t p = 0; p < position_count; ++p) {
            for (size_t t = 0; t < time_count; ++t) {
                for (size_t a = 0; a < activity_count; ++a) {
                    for (size_t s = 0; s < seed_count; ++s) {
                        uint32_t glyph = ascii_art_animated_glyph(
                            &cells[c], rows[p], columns[p],
                            times[t], activities[a], seeds[s]);
                        printf("anim %zu %zu %zu %zu %zu %u\n",
                               c, p, t, a, s, (unsigned)glyph);
                    }
                }
            }
        }
    }
}

int main(void)
{
    dump_fit();
    dump_layout();
    dump_populated();
    dump_convert();
    dump_rejections();
    dump_divergences();
    dump_grids();
    dump_anim();
    return 0;
}
