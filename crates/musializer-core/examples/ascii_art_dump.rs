//! Dumps the Rust `ascii_art` module's output for a set of deterministic
//! synthetic pixel buffers, in the same format as
//! `tests/differential/ascii_art_oracle.c`.
//!
//! Run through `tools/differential_ascii_art.sh`, which builds the oracle's
//! `ascii_art.c` from the frozen C source, runs both, and compares numerically.
//! The module is pure — RGBA8 pixels in, a grid of glyph cells out — so a number
//! settles it rather than a paragraph about whether the port is faithful.
//!
//! An `examples/` target rather than a `[[bin]]` on purpose: examples need no
//! manifest entry, so this does not touch a file another agent might be editing.
//!
//! The pixel fixtures are duplicated between here and the C harness
//! deliberately. Sharing them would let a bug hide in the shared half. They are
//! integer-only arithmetic, so "both sides saw identical input" needs no float
//! agreement to establish.
//!
//! Floats print as `{:.8e}` — nine significant digits in exponential form —
//! rather than a fixed number of decimals, because the layout cases reach 1e30
//! and a fixed-point spelling there is not comparable with C's `%.9g`.

use musializer_core::scenes::ascii_field::ascii_art::{
    self, Cell, EdgeOrientation, GRID_MAX_CELLS, GRID_MAX_COLUMNS, GRID_MAX_ROWS,
};

/// 128x72 is the largest source any case uses.
const MAX_PIXEL_BYTES: usize = 128 * 72 * 4;

/// Printed back after a rejected call, so a write on the failure path shows up as
/// a diff rather than passing silently.
const FIT_SENTINEL: usize = 999_999;
const LAYOUT_SENTINEL: f32 = -12345.0;
const CELL_SENTINEL: u32 = 0x1234_5678;

// ---------------------------------------------------------------------------
// Fixtures. The second copy of the arithmetic in `ascii_art_oracle.c`.
// ---------------------------------------------------------------------------

fn fill_fixture(kind: u32, pixels: &mut [u8], width: usize, height: usize) {
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let (r, g, b, a): (u32, u32, u32, u32) = match kind {
                // Smooth two-axis gradient with a quadratic blue channel.
                0 => (
                    ((x * 7 + y * 3) & 0xff) as u32,
                    ((x * 13 + y * 29) & 0xff) as u32,
                    (((x * x + y * y) * 5) & 0xff) as u32,
                    255,
                ),
                // Hard quadrant edges with a diagonal overlay.
                1 => {
                    let mut value: u32 = 0;
                    if x >= width / 2 {
                        value += 160;
                    }
                    if y >= height / 2 {
                        value += 80;
                    }
                    if (x + y) % 5 == 0 {
                        value = 255 - value;
                    }
                    (value, value, value, 255)
                }
                // Varying alpha, including fully transparent pixels.
                2 => (
                    ((x * 31) & 0xff) as u32,
                    ((y * 47) & 0xff) as u32,
                    (((x + y) * 11) & 0xff) as u32,
                    if (x * 37 + y * 53) % 6 == 0 {
                        0
                    } else {
                        ((x * 17 + y * 23) & 0xff) as u32
                    },
                ),
                // Byte noise across all four channels, alpha included.
                3 => (
                    (((index * 4) * 73 + 19) & 0xff) as u32,
                    (((index * 4 + 1) * 73 + 19) & 0xff) as u32,
                    (((index * 4 + 2) * 73 + 19) & 0xff) as u32,
                    (((index * 4 + 3) * 73 + 19) & 0xff) as u32,
                ),
                // Flat dark grey: the sparse-ink lift with no edges at all.
                4 => (32, 32, 32, 255),
                // Checkerboard: maximum gradient everywhere, no coherence.
                5 => {
                    let value = if ((x + y) & 1) == 1 { 255 } else { 0 };
                    (value, value, value, 255)
                }
                // A 24-wide luminance band: the no-op contrast stretch.
                6 => {
                    let value = (40 + (x * 3 + y * 5) % 24) as u32;
                    (value, value, value, 255)
                }
                // Fully transparent: the alpha == 0 colour branch.
                7 => (
                    ((x * 9) & 0xff) as u32,
                    ((y * 11) & 0xff) as u32,
                    (((x ^ y) * 13) & 0xff) as u32,
                    0,
                ),
                // One bright pixel: the percentile edges of the histogram.
                8 => {
                    let value = if x == width / 2 && y == height / 2 {
                        255
                    } else {
                        0
                    };
                    (value, value, value, 255)
                }
                _ => (0, 0, 0, 255),
            };
            pixels[index * 4] = r as u8;
            pixels[index * 4 + 1] = g as u8;
            pixels[index * 4 + 2] = b as u8;
            pixels[index * 4 + 3] = a as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// fit
// ---------------------------------------------------------------------------

#[rustfmt::skip]
const FIT_SOURCES: &[(usize, usize)] = &[
    (0, 0), (0, 100), (100, 0),
    (1, 1), (1, 2), (2, 1), (3, 1), (1, 3),
    (16, 9), (9, 16), (40, 20), (20, 40),
    (96, 54), (54, 96),
    (1408, 768), (768, 1408),
    (1920, 1080), (1080, 1920),
    (3840, 2160), (2160, 3840),
    (10000, 1), (1, 10000),
    (100000, 3), (3, 100000),
    (65536, 65535), (65535, 65536),
    (12345, 6789), (6789, 12345),
    (usize::MAX, 1), (1, usize::MAX),
    (usize::MAX, usize::MAX), (usize::MAX, 2), (2, usize::MAX),
    (usize::MAX / 2, 3), (3, usize::MAX / 2),
];

#[rustfmt::skip]
const FIT_MAXIMA: &[(usize, usize)] = &[
    (0, 0), (0, 54), (96, 0),
    (1, 1), (2, 1), (1, 2),
    (96, 54), (320, 200), (40, 20),
    (usize::MAX, usize::MAX), (usize::MAX, 54), (96, usize::MAX),
];

fn dump_fit() {
    for &(source_width, source_height) in FIT_SOURCES {
        for &(maximum_columns, maximum_rows) in FIT_MAXIMA {
            let fitted = ascii_art::fit_grid_dimensions(
                source_width,
                source_height,
                maximum_columns,
                maximum_rows,
            );
            let (ok, columns, rows) = match fitted {
                Some((columns, rows)) => (1, columns, rows),
                None => (0, FIT_SENTINEL, FIT_SENTINEL),
            };
            println!(
                "fit {source_width} {source_height} {maximum_columns} {maximum_rows} \
                 {ok} {columns} {rows}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

#[rustfmt::skip]
fn layout_cases() -> Vec<(f32, f32, usize, usize, f32)> {
    vec![
        (640.0, 360.0, 96, 52, 1.0),
        (1280.0, 720.0, 96, 52, 1.0),
        (640.0, 360.0, 96, 26, 0.5),
        (1920.0, 1080.0, 96, 54, 1.0),
        (100.0, 100.0, 1, 1, 1.0),
        (3.5, 7.25, 7, 3, 2.0),
        (1000.0, 10.0, 96, 54, 1.0),
        (10.0, 1000.0, 96, 54, 1.0),
        (1e30, 1e30, 96, 54, 1.0),
        (1e-30, 1e-30, 96, 54, 1.0),
        (640.0, 360.0, i32::MAX as usize, 52, 1.0),
        // rejections
        (100.0, 100.0, 0, 10, 1.0),
        (100.0, 100.0, 10, 0, 1.0),
        (f32::NAN, 360.0, 96, 52, 1.0),
        (640.0, f32::NAN, 96, 52, 1.0),
        (640.0, 360.0, 96, 52, f32::NAN),
        (f32::INFINITY, 360.0, 96, 52, 1.0),
        (0.0, 360.0, 96, 52, 1.0),
        (640.0, -1.0, 96, 52, 1.0),
        (640.0, 360.0, 96, 52, 0.0),
        (640.0, 360.0, 96, 52, -1.0),
        (640.0, 360.0, i32::MAX as usize + 1, 52, 1.0),
    ]
}

fn dump_layout() {
    for (index, (area_width, area_height, columns, rows, cell_aspect)) in
        layout_cases().into_iter().enumerate()
    {
        let laid_out = ascii_art::grid_layout(area_width, area_height, columns, rows, cell_aspect);
        let sentinel = ascii_art::GridLayout {
            cell_width: LAYOUT_SENTINEL,
            cell_height: LAYOUT_SENTINEL,
            field_width: LAYOUT_SENTINEL,
            field_height: LAYOUT_SENTINEL,
            offset_x: LAYOUT_SENTINEL,
            offset_y: LAYOUT_SENTINEL,
        };
        let ok = i32::from(laid_out.is_some());
        let l = laid_out.unwrap_or(sentinel);
        println!(
            "layout {index} {ok} {:.8e} {:.8e} {:.8e} {:.8e} {:.8e} {:.8e}",
            l.cell_width, l.cell_height, l.field_width, l.field_height, l.offset_x, l.offset_y,
        );
    }
}

// ---------------------------------------------------------------------------
// populated
// ---------------------------------------------------------------------------

fn dump_populated() {
    #[rustfmt::skip]
    let cases: &[(usize, usize)] = &[
        (0, 0), (0, 1), (1, 0), (1, 1), (96, 54),
        (usize::MAX, 1), (1, usize::MAX), (usize::MAX, usize::MAX), (0, usize::MAX),
    ];
    for &(width, height) in cases {
        let populated = i32::from(ascii_art::grid_is_populated(width, height));
        println!("populated {width} {height} {populated}");
    }
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

/// `(kind, source_width, source_height, grid_width, grid_height)`.
#[rustfmt::skip]
const CONVERT_CASES: &[(u32, usize, usize, usize, usize)] = &[
    (0, 1, 1, 1, 1),
    (0, 2, 2, 1, 1),
    (0, 2, 2, 2, 2),
    (0, 3, 3, 2, 2),      // Bresenham with a remainder in both axes
    (0, 5, 5, 5, 5),
    (0, 5, 5, 2, 2),
    (0, 7, 5, 3, 2),
    (0, 37, 23, 11, 7),
    (0, 64, 1, 4, 1),
    (0, 1, 64, 1, 4),
    (0, 96, 54, 1, 1),
    (0, 128, 72, 96, 54),
    (1, 5, 5, 1, 1),
    (1, 7, 7, 1, 1),
    (1, 9, 9, 1, 1),
    (1, 16, 9, 16, 9),
    (1, 128, 72, 32, 18),
    (1, 128, 72, 96, 54),
    (2, 8, 6, 4, 3),
    (2, 37, 23, 12, 8),
    (2, 96, 54, 96, 54),
    (3, 8, 6, 4, 3),
    (3, 64, 64, 21, 21),
    (3, 100, 60, 96, 54),
    (4, 1, 1, 1, 1),
    (4, 16, 16, 4, 4),
    (5, 9, 9, 3, 3),
    (5, 64, 36, 64, 36),
    (5, 65, 37, 32, 18),
    (6, 64, 1, 4, 1),
    (6, 40, 30, 20, 15),
    (6, 40, 30, 7, 5),
    (7, 4, 4, 2, 2),
    (7, 33, 17, 11, 5),
    (8, 11, 11, 11, 11),
    (8, 11, 11, 1, 1),
];

fn sentinel_cell() -> Cell {
    Cell {
        glyph: CELL_SENTINEL,
        ..Cell::default()
    }
}

fn print_cell(record: &str, case_index: usize, cell_index: usize, cell: &Cell) {
    println!(
        "{record} {case_index} {cell_index} {} {} {} {} {} {:.8e} {:.8e} {}",
        cell.glyph,
        cell.foreground.r,
        cell.foreground.g,
        cell.foreground.b,
        cell.foreground.a,
        cell.luminance,
        cell.edge_strength,
        cell.edge_orientation as u8,
    );
}

fn dump_convert(pixels: &mut [u8]) {
    for (index, &(kind, source_width, source_height, grid_width, grid_height)) in
        CONVERT_CASES.iter().enumerate()
    {
        let cells_wanted = grid_width * grid_height;
        fill_fixture(kind, pixels, source_width, source_height);
        let mut cells = vec![sentinel_cell(); GRID_MAX_CELLS + 8];
        let ok = ascii_art::convert_rgba8(
            &pixels[..source_width * source_height * 4],
            source_width,
            source_height,
            grid_width,
            grid_height,
            &mut cells[..cells_wanted],
        );
        println!(
            "convert {index} {kind} {source_width} {source_height} {grid_width} {grid_height} {} {cells_wanted}",
            i32::from(ok)
        );
        if !ok {
            continue;
        }
        for (cell_index, cell) in cells[..cells_wanted].iter().enumerate() {
            print_cell("cell", index, cell_index, cell);
        }
    }
}

// ---------------------------------------------------------------------------
// convert rejections
// ---------------------------------------------------------------------------

/// `(label, source_width, source_height, grid_width, grid_height, output_count)`.
#[rustfmt::skip]
const REJECT_CASES: &[(&str, usize, usize, usize, usize, usize)] = &[
    ("zero_source_width", 0, 1, 1, 1, 2),
    ("zero_source_height", 1, 0, 1, 1, 2),
    ("zero_grid_width", 1, 1, 0, 1, 2),
    ("zero_grid_height", 1, 1, 1, 0, 2),
    ("grid_wider_than_source", 1, 1, 2, 1, 2),
    ("grid_taller_than_source", 1, 1, 1, 2, 2),
    ("output_too_small", 2, 2, 2, 2, 3),
    ("output_empty", 1, 1, 1, 1, 0),
    ("source_dimensions_overflow", usize::MAX, 2, 1, 1, 2),
    ("source_count_accumulator_overflow", 16_777_216, 16_777_216, 1, 1, 2),
];

fn dump_rejections(pixels: &mut [u8]) {
    fill_fixture(0, pixels, 2, 2);
    for &(label, source_width, source_height, grid_width, grid_height, output_count) in REJECT_CASES
    {
        let mut cells = [sentinel_cell(); 4];
        let ok = ascii_art::convert_rgba8(
            &pixels[..16],
            source_width,
            source_height,
            grid_width,
            grid_height,
            &mut cells[..output_count],
        );
        // The two guards prove atomicity: a rejected call must leave the
        // caller's buffer exactly as it found it.
        println!(
            "reject {label} {} {} {}",
            i32::from(ok),
            cells[0].glyph,
            cells[1].glyph
        );
    }
}

// ---------------------------------------------------------------------------
// Divergences in mechanism, recorded rather than hidden. Slices cannot express a
// null pointer, and they *can* detect a pixel buffer shorter than the dimensions
// claim; the C's bare pointers are the other way round. Both halves print their
// side of each pair and the driver asserts the pair, which keeps the difference
// visible instead of untested.
// ---------------------------------------------------------------------------

fn dump_divergences(pixels: &mut [u8]) {
    println!("divergence null_pixels not_expressible");
    println!("divergence null_output not_expressible");
    println!("divergence null_cell not_expressible");
    fill_fixture(0, pixels, 2, 2);
    let mut cells = [sentinel_cell(); 4];
    // Fifteen bytes for a 2x2 RGBA8 image. C would read the sixteenth anyway.
    let ok = ascii_art::convert_rgba8(&pixels[..15], 2, 2, 2, 2, &mut cells);
    println!(
        "divergence truncated_pixels {}",
        if ok { "rust_accepts" } else { "rust_rejects" }
    );
    assert_eq!(
        cells[0].glyph, CELL_SENTINEL,
        "a rejected conversion must not touch the output"
    );
}

// ---------------------------------------------------------------------------
// grid: `Grid::from_rgba8`, which replaced plug.c:860-891's fit-then-convert
// composition. This is the path `--ascii-image` runs.
// ---------------------------------------------------------------------------

/// `(kind, source_width, source_height)`.
#[rustfmt::skip]
const GRID_CASES: &[(u32, usize, usize)] = &[
    (0, 128, 72),
    (1, 100, 60),
    (2, 40, 120),
    (3, 8, 6),
    (5, 65, 37),
    (8, 11, 11),
    (4, 3840, 1),   // a 4K-wide strip must still fit the fixed 96x54 buffer
];

fn dump_grids(pixels: &mut [u8]) {
    for (index, &(kind, source_width, source_height)) in GRID_CASES.iter().enumerate() {
        let bytes = source_width * source_height * 4;
        assert!(
            bytes <= MAX_PIXEL_BYTES,
            "grid case {index} needs {bytes} bytes"
        );
        fill_fixture(kind, pixels, source_width, source_height);
        let grid = ascii_art::Grid::from_rgba8(
            &pixels[..bytes],
            source_width,
            source_height,
            GRID_MAX_COLUMNS,
            GRID_MAX_ROWS,
        );
        let (ok, columns, rows) = match &grid {
            Some(grid) => (1, grid.columns(), grid.rows()),
            None => (0, 0, 0),
        };
        println!("grid {index} {kind} {source_width} {source_height} {ok} {columns} {rows}");
        let Some(grid) = grid else { continue };
        for (cell_index, cell) in grid.cells().iter().enumerate() {
            print_cell("gridcell", index, cell_index, cell);
        }
    }
}

// ---------------------------------------------------------------------------
// anim. Every axis is an index into a table duplicated in the C harness, so a
// NaN or an infinity can be driven without having to print one.
// ---------------------------------------------------------------------------

fn dump_anim() {
    let cells = [
        cell(u32::from(b'L'), 0.58, 0.0, EdgeOrientation::None),
        cell(u32::from(b'G'), 0.72, 0.0, EdgeOrientation::None),
        cell(u32::from(b' '), 0.0, 0.0, EdgeOrientation::None),
        cell(u32::from(b' '), 0.5, 0.0, EdgeOrientation::None),
        cell(u32::from(b'/'), 0.58, 0.4, EdgeOrientation::DiagonalForward),
        // An out-of-range codepoint on an edge cell: substituted with U+FFFD.
        cell(0x0011_0001, 0.5, 0.4, EdgeOrientation::Horizontal),
        cell(u32::from(b'@'), 1.0, 0.0, EdgeOrientation::None),
        cell(u32::from(b'.'), 0.02, 0.0, EdgeOrientation::None),
        cell(u32::from(b'C'), f32::NAN, 0.0, EdgeOrientation::None),
        cell(u32::from(b'#'), 1.5, 0.0, EdgeOrientation::None),
        cell(u32::from(b'8'), -0.3, 0.0, EdgeOrientation::None),
    ];

    let rows = [0usize, 7, 19, 53];
    let columns = [0usize, 11, 31, 95];
    #[rustfmt::skip]
    let times = [
        0.0f64, 0.0625, 0.125, 0.25, 0.5, 1.0, 1.5, 2.75,
        7.125, 47.125, 123.5, 4096.0, 4097.25, -3.5, f64::NAN, f64::INFINITY,
    ];
    let activities = [0.0f32, 0.37, 1.0, 2.5, f32::NAN];
    let seeds = [0u64, 0x1234, 0xfeed_beef, 0xffff_ffff_ffff_ffff];

    for (c, cell) in cells.iter().enumerate() {
        for (p, (&row, &column)) in rows.iter().zip(columns.iter()).enumerate() {
            for (t, &time) in times.iter().enumerate() {
                for (a, &activity) in activities.iter().enumerate() {
                    for (s, &seed) in seeds.iter().enumerate() {
                        let glyph =
                            ascii_art::animated_glyph(cell, row, column, time, activity, seed);
                        println!("anim {c} {p} {t} {a} {s} {glyph}");
                    }
                }
            }
        }
    }
}

fn cell(glyph: u32, luminance: f32, edge_strength: f32, edge_orientation: EdgeOrientation) -> Cell {
    Cell {
        glyph,
        foreground: ascii_art::Rgba::default(),
        luminance,
        edge_strength,
        edge_orientation,
    }
}

fn main() {
    let mut pixels = vec![0u8; MAX_PIXEL_BYTES];
    dump_fit();
    dump_layout();
    dump_populated();
    dump_convert(&mut pixels);
    dump_rejections(&mut pixels);
    dump_divergences(&mut pixels);
    dump_grids(&mut pixels);
    dump_anim();
}
