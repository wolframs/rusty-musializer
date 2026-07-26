//! Image-to-ASCII conversion: grid fitting, layout, RGBA8 conversion, and the
//! deterministic glyph animation.
//!
//! **Owner: Agent C.** Port of `../musializer/src/ascii_art.c` and `.h` (frozen
//! at `9300af9`, read-only).
//!
//! Raylib-free in C and raylib-free here, which is why it has tests: the C suite
//! covers it in `../musializer/tests/test_ascii_art.c` and all fourteen of those
//! tests are ported at the bottom of this file.
//!
//! Two habits from the C carried over deliberately:
//!
//! - **Integer arithmetic throughout the conversion.** Luminance, colour
//!   averaging and the edge classifier are integer maths so the result is byte
//!   reproducible on any target, which one of the C tests pins.
//! - **Atomic outputs.** C replaces its out-parameters only after a complete fit
//!   is found. In Rust that falls out of returning `Option`, and the tests that
//!   check a sentinel survives a rejected call are kept as tests that nothing
//!   was written.

// C's guard clauses are kept as C's chains of comparisons rather than rewritten
// as range `contains` calls, so each guard diffs against the line it came from.
// Reverting these is a refactor for after the application works.
#![allow(clippy::manual_range_contains)]

/// Dark to light. More intermediate ink densities retain faces and line art
/// (`ascii_art.c:8`).
const TONE_RAMP: &[u8; 16] = b" .,:;i1tfLCG08#@";

/// `ASCII_GRID_MAX_COLUMNS` (`ascii_art.h:9`).
pub const GRID_MAX_COLUMNS: usize = 96;
/// `ASCII_GRID_MAX_ROWS` (`ascii_art.h:10`).
pub const GRID_MAX_ROWS: usize = 54;
/// `ASCII_GRID_MAX_CELLS` (`ascii_art.h:11`).
pub const GRID_MAX_CELLS: usize = GRID_MAX_COLUMNS * GRID_MAX_ROWS;

/// `AsciiRgba` (`ascii_art.h:14-19`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// `AsciiEdgeOrientation` (`ascii_art.h:21-27`).
///
/// The discriminants are load-bearing: `classify_orientation` indexes an energy
/// array by orientation and the dominance scan walks `Horizontal..=DiagonalBack`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EdgeOrientation {
    #[default]
    None = 0,
    Horizontal = 1,
    Vertical = 2,
    DiagonalForward = 3,
    DiagonalBack = 4,
}

impl EdgeOrientation {
    /// The four real orientations, in the C enum's order.
    const CANDIDATES: [EdgeOrientation; 4] = [
        EdgeOrientation::Horizontal,
        EdgeOrientation::Vertical,
        EdgeOrientation::DiagonalForward,
        EdgeOrientation::DiagonalBack,
    ];

    /// `glyph_for_edge` (`ascii_art.c:230-240`). `None` has no glyph, which is
    /// C's `0`.
    #[must_use]
    pub fn glyph(self) -> u32 {
        match self {
            EdgeOrientation::Horizontal => u32::from(b'-'),
            EdgeOrientation::Vertical => u32::from(b'|'),
            EdgeOrientation::DiagonalForward => u32::from(b'/'),
            EdgeOrientation::DiagonalBack => u32::from(b'\\'),
            EdgeOrientation::None => 0,
        }
    }
}

/// One converted cell (`AsciiCell`, `ascii_art.h:29-36`).
///
/// `Default` is C's zeroed cell: glyph 0, black transparent foreground, no
/// luminance and no edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// Unicode codepoint. The built-in ramp and edge glyphs are all ASCII.
    pub glyph: u32,
    pub foreground: Rgba,
    pub luminance: f32,
    pub edge_strength: f32,
    pub edge_orientation: EdgeOrientation,
}

/// A centred grid inside a drawing area (`AsciiGridLayout`, `ascii_art.h:38-45`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridLayout {
    pub cell_width: f32,
    pub cell_height: f32,
    pub field_width: f32,
    pub field_height: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

fn clamp01(value: f32) -> f32 {
    // `clamp01` (`ascii_art.c:10-15`): NaN reads as 0.
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    if value >= 1.0 {
        return 1.0;
    }
    value
}

/// `seed_phase` (`ascii_art.c:17-26`): splitmix64's finalizer, low 16 bits, times
/// tau.
fn seed_phase(seed: u64) -> f32 {
    // C spells this `6.28318530717958647692f`, which rounds to exactly
    // `f32::consts::TAU`.
    const TAU: f32 = std::f32::consts::TAU;
    let mut seed = seed;
    seed ^= seed >> 30;
    seed = seed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    seed ^= seed >> 27;
    seed = seed.wrapping_mul(0x94d0_49bb_1331_11eb);
    seed ^= seed >> 31;
    (seed & 0xffff) as f32 / 65535.0 * TAU
}

/// `multiply_add_half` (`ascii_art.c:28-40`): `(value*multiplier + divisor/2)/divisor`
/// with the overflow checks C needs.
fn multiply_add_half(value: usize, multiplier: usize, divisor: usize) -> Option<usize> {
    if divisor == 0 || multiplier == 0 {
        return None;
    }
    let product = value.checked_mul(multiplier)?;
    let half = divisor / 2;
    Some(product.checked_add(half)? / divisor)
}

/// Fits a source image into a bounded grid without changing its sample aspect
/// (`ascii_art_fit_grid_dimensions`, `ascii_art.c:42-72`).
///
/// Never upscales beyond the source dimensions. Returns `(columns, rows)`.
#[must_use]
pub fn fit_grid_dimensions(
    source_width: usize,
    source_height: usize,
    maximum_columns: usize,
    maximum_rows: usize,
) -> Option<(usize, usize)> {
    if source_width == 0 || source_height == 0 || maximum_columns == 0 || maximum_rows == 0 {
        return None;
    }

    let mut fitted_columns = source_width.min(maximum_columns);
    let mut fitted_rows = multiply_add_half(source_height, fitted_columns, source_width)?;
    if fitted_rows < 1 {
        fitted_rows = 1;
    }

    // Too tall for the box: pin rows instead and re-derive columns from them.
    if fitted_rows > maximum_rows {
        fitted_rows = source_height.min(maximum_rows);
        fitted_columns = multiply_add_half(source_width, fitted_rows, source_height)?;
        if fitted_columns < 1 {
            fitted_columns = 1;
        }
    }
    if fitted_columns > maximum_columns
        || fitted_rows > maximum_rows
        || fitted_columns > source_width
        || fitted_rows > source_height
    {
        return None;
    }
    Some((fitted_columns, fitted_rows))
}

/// Centres a grid inside a drawing area (`ascii_art_grid_layout`,
/// `ascii_art.c:74-107`).
///
/// `cell_aspect` is cell width/height: 1 for square samples, 0.5 to display a
/// legacy half-height ASCII grid.
#[must_use]
pub fn grid_layout(
    area_width: f32,
    area_height: f32,
    columns: usize,
    rows: usize,
    cell_aspect: f32,
) -> Option<GridLayout> {
    if columns == 0
        || rows == 0
        || !area_width.is_finite()
        || !area_height.is_finite()
        || !cell_aspect.is_finite()
        || area_width <= 0.0
        || area_height <= 0.0
        || cell_aspect <= 0.0
        || columns > i32::MAX as usize
        || rows > i32::MAX as usize
    {
        return None;
    }

    let by_width = area_width / (columns as f32 * cell_aspect);
    let by_height = area_height / rows as f32;
    let cell_height = by_width.min(by_height);
    let cell_width = cell_height * cell_aspect;
    let field_width = cell_width * columns as f32;
    let field_height = cell_height * rows as f32;
    if !cell_width.is_finite()
        || !cell_height.is_finite()
        || !field_width.is_finite()
        || !field_height.is_finite()
        || cell_width <= 0.0
        || cell_height <= 0.0
    {
        return None;
    }

    Some(GridLayout {
        cell_width,
        cell_height,
        field_width,
        field_height,
        offset_x: (area_width - field_width) * 0.5,
        offset_y: (area_height - field_height) * 0.5,
    })
}

/// `luminance_u8` (`ascii_art.c:109-113`): integer Rec. 709, coefficients summing
/// to 256.
fn luminance_u8(pixel: &[u8]) -> u8 {
    ((54 * u32::from(pixel[0]) + 183 * u32::from(pixel[1]) + 19 * u32::from(pixel[2]) + 128) >> 8)
        as u8
}

/// `visible_luminance_u8` (`ascii_art.c:115-119`): luminance premultiplied by
/// alpha, so a transparent pixel contributes no ink.
fn visible_luminance_u8(pixel: &[u8]) -> u8 {
    let luminance = u32::from(luminance_u8(pixel));
    ((luminance * u32::from(pixel[3]) + 127) / 255) as u8
}

fn source_offset(x: usize, y: usize, width: usize) -> usize {
    (y * width + x) * 4
}

/// `sample_luminance` (`ascii_art.c:126-150`): a neighbour sample clamped at the
/// image edge.
///
/// The branch structure is C's verbatim, including the way `dx > 0` at the right
/// edge falls through every arm and leaves `x` alone — that clamping is what the
/// Sobel operator sees on the border.
fn sample_luminance(
    pixels: &[u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    dx: i32,
    dy: i32,
) -> u8 {
    let mut x = x;
    let mut y = y;
    if dx < 0 && x == 0 {
        x = 0;
    } else if dx > 0 && x + 1 < width {
        x += 1;
    } else if dx < 0 {
        x -= 1;
    }

    if dy < 0 && y == 0 {
        y = 0;
    } else if dy > 0 && y + 1 < height {
        y += 1;
    } else if dy < 0 {
        y -= 1;
    }
    let offset = source_offset(x, y, width);
    visible_luminance_u8(&pixels[offset..offset + 4])
}

/// `sobel_at` (`ascii_art.c:152-173`).
fn sobel_at(pixels: &[u8], width: usize, height: usize, x: usize, y: usize) -> (i32, i32) {
    let at = |dx, dy| i32::from(sample_luminance(pixels, width, height, x, y, dx, dy));
    let upper_left = at(-1, -1);
    let upper = at(0, -1);
    let upper_right = at(1, -1);
    let left = at(-1, 0);
    let right = at(1, 0);
    let lower_left = at(-1, 1);
    let lower = at(0, 1);
    let lower_right = at(1, 1);

    let gradient_x = -upper_left + upper_right - 2 * left + 2 * right - lower_left + lower_right;
    let gradient_y = -upper_left - 2 * upper - upper_right + lower_left + 2 * lower + lower_right;
    (gradient_x, gradient_y)
}

/// `classify_orientation` (`ascii_art.c:185-196`).
///
/// A gradient more than twice as strong in one axis is an axis-aligned edge;
/// otherwise the *signs* pick the diagonal. Equal-and-opposite signs read as a
/// back-slash.
fn classify_orientation(gradient_x: i64, gradient_y: i64) -> EdgeOrientation {
    let x = gradient_x.unsigned_abs();
    let y = gradient_y.unsigned_abs();
    if x == 0 && y == 0 {
        return EdgeOrientation::None;
    }
    if x > 2 * y {
        return EdgeOrientation::Vertical;
    }
    if y > 2 * x {
        return EdgeOrientation::Horizontal;
    }
    if (gradient_x < 0) == (gradient_y < 0) {
        EdgeOrientation::DiagonalForward
    } else {
        EdgeOrientation::DiagonalBack
    }
}

/// `histogram_percentile` (`ascii_art.c:198-208`).
fn histogram_percentile(histogram: &[usize; 256], rank: usize) -> u8 {
    let mut cumulative = 0usize;
    for (value, &count) in histogram.iter().enumerate() {
        match cumulative.checked_add(count) {
            None => return value as u8,
            Some(next) => cumulative = next,
        }
        if cumulative > rank {
            return value as u8;
        }
    }
    255
}

/// `contrast_luminance` (`ascii_art.c:210-219`): a robust black/white stretch.
///
/// A span of 24 or less is left untouched, so a nearly flat image is not blown up
/// into noise.
fn contrast_luminance(luminance: u8, black_point: u8, white_point: u8) -> u8 {
    if u32::from(white_point) <= u32::from(black_point) + 24 {
        return luminance;
    }
    if luminance <= black_point {
        return 0;
    }
    if luminance >= white_point {
        return 255;
    }
    let span = u32::from(white_point) - u32::from(black_point);
    ((u32::from(luminance - black_point) * 255 + span / 2) / span) as u8
}

/// `glyph_density_luminance` (`ascii_art.c:221-228`).
///
/// Character ramps put far less ink on screen than a filled image pixel, so a
/// mild perceptual lift keeps shadow structure visible without moving either
/// endpoint or flattening the source contrast.
fn glyph_density_luminance(luminance: u32) -> u32 {
    let normalized = luminance as f32 / 255.0;
    (normalized.powf(0.72) * 255.0 + 0.5) as u32
}

/// `dimensions_valid` (`ascii_art.c:242-262`).
///
/// The three `UINT64_MAX/...` tests bound the accumulators used per cell — the
/// alpha-weighted colour sums and the `2040*pixel_count` edge denominator. They
/// are kept because they are the reason a hostile image cannot overflow them.
fn dimensions_valid(
    source_width: usize,
    source_height: usize,
    grid_width: usize,
    grid_height: usize,
    output_count: usize,
) -> bool {
    if source_width == 0
        || source_height == 0
        || grid_width == 0
        || grid_height == 0
        || grid_width > source_width
        || grid_height > source_height
    {
        return false;
    }
    let Some(source_count) = source_width.checked_mul(source_height) else {
        return false;
    };
    let source_count = source_count as u64;
    if source_count > usize::MAX as u64 / 4
        || source_count > u64::MAX / (2040 * 100)
        || source_count > u64::MAX / 65025
    {
        return false;
    }
    let Some(cells) = grid_width.checked_mul(grid_height) else {
        return false;
    };
    output_count >= cells
}

/// Converts tightly packed, row-major RGBA8 pixels into a row-major ASCII grid
/// (`ascii_art_convert_rgba8`, `ascii_art.c:264-393`).
///
/// Each output cell averages a non-empty rectangular source region, so the grid
/// may not be larger than the source in either axis. Returns `false` without
/// touching `output` when anything is invalid.
///
/// The Bresenham-style `x_error`/`y_error` walk is kept rather than replaced with
/// a division per cell: it is what makes the region boundaries land exactly where
/// the C's do, and cell boundaries are visible in the result.
pub fn convert_rgba8(
    pixels: &[u8],
    source_width: usize,
    source_height: usize,
    grid_width: usize,
    grid_height: usize,
    output: &mut [Cell],
) -> bool {
    if !dimensions_valid(
        source_width,
        source_height,
        grid_width,
        grid_height,
        output.len(),
    ) {
        return false;
    }
    let source_count = source_width * source_height;
    if pixels.len() < source_count * 4 {
        // C trusts the caller's buffer; Rust cannot, so a short slice is a
        // rejection rather than a read past the end.
        return false;
    }

    let mut histogram = [0usize; 256];
    for index in 0..source_count {
        let luminance = visible_luminance_u8(&pixels[index * 4..index * 4 + 4]);
        histogram[usize::from(luminance)] += 1;
    }
    // The robust black and white points: the 1st and 99th percentiles of visible
    // luminance, so a few blown highlights do not set the whole exposure.
    let trim = source_count / 100;
    let black_point = histogram_percentile(&histogram, trim);
    let white_point = histogram_percentile(&histogram, source_count - trim - 1);

    let mut source_y = 0usize;
    let mut y_error = 0usize;
    let y_quotient = source_height / grid_height;
    let y_remainder = source_height % grid_height;
    for cell_y in 0..grid_height {
        let mut next_y = source_y + y_quotient;
        y_error += y_remainder;
        if y_error >= grid_height {
            next_y += 1;
            y_error -= grid_height;
        }

        let mut source_x = 0usize;
        let mut x_error = 0usize;
        let x_quotient = source_width / grid_width;
        let x_remainder = source_width % grid_width;
        for cell_x in 0..grid_width {
            let mut next_x = source_x + x_quotient;
            x_error += x_remainder;
            if x_error >= grid_width {
                next_x += 1;
                x_error -= grid_width;
            }

            let mut weighted_red = 0u64;
            let mut weighted_green = 0u64;
            let mut weighted_blue = 0u64;
            let mut alpha = 0u64;
            let mut luma = 0u64;
            let mut edge_sum = 0u64;
            let mut orientation_energy = [0u64; 5];
            let pixel_count = (next_x - source_x) * (next_y - source_y);

            for y in source_y..next_y {
                for x in source_x..next_x {
                    let offset = source_offset(x, y, source_width);
                    let pixel = &pixels[offset..offset + 4];
                    weighted_red += u64::from(pixel[0]) * u64::from(pixel[3]);
                    weighted_green += u64::from(pixel[1]) * u64::from(pixel[3]);
                    weighted_blue += u64::from(pixel[2]) * u64::from(pixel[3]);
                    alpha += u64::from(pixel[3]);
                    luma += u64::from(visible_luminance_u8(pixel));
                    let (gradient_x, gradient_y) =
                        sobel_at(pixels, source_width, source_height, x, y);
                    let magnitude =
                        u64::from(gradient_x.unsigned_abs()) + u64::from(gradient_y.unsigned_abs());
                    let orientation =
                        classify_orientation(i64::from(gradient_x), i64::from(gradient_y));
                    edge_sum += magnitude;
                    orientation_energy[orientation as usize] += magnitude;
                }
            }

            // Colour is averaged with alpha as the weight, so a fully
            // transparent pixel's RGB never tints the cell.
            let color_weight = if alpha > 0 { alpha } else { pixel_count as u64 };
            let pixel_count_u64 = pixel_count as u64;
            let mut cell = Cell {
                foreground: Rgba {
                    r: if alpha > 0 {
                        ((weighted_red + color_weight / 2) / color_weight) as u8
                    } else {
                        0
                    },
                    g: if alpha > 0 {
                        ((weighted_green + color_weight / 2) / color_weight) as u8
                    } else {
                        0
                    },
                    b: if alpha > 0 {
                        ((weighted_blue + color_weight / 2) / color_weight) as u8
                    } else {
                        0
                    },
                    a: ((alpha + pixel_count_u64 / 2) / pixel_count_u64) as u8,
                },
                edge_strength: edge_sum as f32 / (2040.0 * pixel_count as f32),
                ..Cell::default()
            };

            let mut dominant_energy = 0u64;
            for orientation in EdgeOrientation::CANDIDATES {
                if orientation_energy[orientation as usize] > dominant_energy {
                    dominant_energy = orientation_energy[orientation as usize];
                    cell.edge_orientation = orientation;
                }
            }

            let mean_luminance = ((luma + pixel_count_u64 / 2) / pixel_count_u64) as u8;
            let mut display_luminance = glyph_density_luminance(u32::from(contrast_luminance(
                mean_luminance,
                black_point,
                white_point,
            )));
            let detail_boost =
                ((edge_sum * 88 + 2040 * pixel_count_u64 / 2) / (2040 * pixel_count_u64)) as u32;
            display_luminance += detail_boost;
            if display_luminance > 255 {
                display_luminance = 255;
            }
            cell.luminance = display_luminance as f32 / 255.0;

            // Only coherent gradients become line glyphs. Curves, text, and
            // crossed detail retain their tone instead of collapsing into a
            // randomly selected slash.
            let coherent_edge = edge_sum > 0 && dominant_energy * 100 >= edge_sum * 60;
            if cell.edge_strength >= 0.035
                && coherent_edge
                && cell.edge_orientation != EdgeOrientation::None
            {
                cell.glyph = cell.edge_orientation.glyph();
            } else {
                cell.edge_orientation = EdgeOrientation::None;
                let ramp_count = TONE_RAMP.len();
                let ramp_index = (display_luminance as usize * (ramp_count - 1) + 127) / 255;
                cell.glyph = u32::from(TONE_RAMP[ramp_index]);
            }
            output[cell_y * grid_width + cell_x] = cell;
            source_x = next_x;
        }
        source_y = next_y;
    }
    true
}

/// Selects a deterministic, time-varying tone glyph for an imported cell
/// (`ascii_art_animated_glyph`, `ascii_art.c:395-431`).
///
/// Coherent edge glyphs and empty background cells stay put, so animation does
/// not erase the source silhouette. `activity` is clamped to `0..=1`; a
/// non-finite `time_seconds` reads as 0, which is what makes the result
/// seek-deterministic rather than history-dependent.
#[must_use]
pub fn animated_glyph(
    cell: &Cell,
    row: usize,
    column: usize,
    time_seconds: f64,
    activity: f32,
    seed: u64,
) -> u32 {
    if cell.edge_orientation != EdgeOrientation::None {
        // The replacement character rather than an invalid codepoint: a cell can
        // only carry a bad glyph if something upstream corrupted it.
        return if cell.glyph <= 0x10_ffff {
            cell.glyph
        } else {
            0xfffd
        };
    }
    if cell.glyph == u32::from(b' ') && clamp01(cell.luminance) < 0.04 {
        return u32::from(b' ');
    }

    let time = if time_seconds.is_finite() {
        (time_seconds % 4096.0) as f32
    } else {
        0.0
    };
    let response = clamp01(activity);
    let phase = seed_phase(seed) + column as f32 * 0.17 - row as f32 * 0.29;
    // A coherent diagonal wave advances through adjacent cells. A slower
    // counter-wave stops the entire image changing glyph on the same frame.
    let primary = (time * (1.10 + response * 0.72) + phase).sin();
    let counter = (time * 0.41 - column as f32 * 0.055 - row as f32 * 0.083
        + seed_phase(seed ^ 0xa076_1d64_78bd_642f))
    .sin();
    let displacement = primary * (0.82 + response * 1.35) + counter * 0.34;

    let ramp_count = TONE_RAMP.len() as i32;
    let luminance = clamp01(cell.luminance);
    let base = (luminance * (ramp_count - 1) as f32 + 0.5).floor() as i32;
    let offset = if displacement >= 0.0 {
        (displacement + 0.5).floor() as i32
    } else {
        (displacement - 0.5).ceil() as i32
    };
    let mut animated = base + offset;
    // A lit cell never animates all the way down to the blank ramp entry, so
    // structure cannot flicker out of existence.
    if animated < 1 && base > 0 {
        animated = 1;
    }
    if animated < 0 {
        animated = 0;
    }
    if animated >= ramp_count {
        animated = ramp_count - 1;
    }
    u32::from(TONE_RAMP[animated as usize])
}

/// A grid is populated only when both dimensions are non-zero
/// (`ascii_art_grid_is_populated`, `ascii_art.c:433-436`).
#[must_use]
pub fn grid_is_populated(grid_width: usize, grid_height: usize) -> bool {
    grid_width > 0 && grid_height > 0
}

/// An imported ASCII grid: cells plus the dimensions they are addressed by.
///
/// C keeps this as three separate fields on `Plug` plus a fixed
/// `AsciiCell[ASCII_GRID_MAX_CELLS]` buffer, with `ascii_art_grid_clear`
/// (`ascii_art.c:438-454`) as the only way to discard one. Bundling them makes
/// the "dimensions agree with the buffer" invariant unbreakable instead of
/// merely maintained, and the bound is still the C's: `columns*rows` can never
/// exceed [`GRID_MAX_CELLS`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    cells: Vec<Cell>,
    columns: usize,
    rows: usize,
}

impl Grid {
    /// Fits and converts an RGBA8 image in one step, the sequence
    /// `--ascii-image` performs.
    ///
    /// Returns `None` if the image cannot be fitted or converted; nothing is
    /// partially built.
    #[must_use]
    pub fn from_rgba8(
        pixels: &[u8],
        source_width: usize,
        source_height: usize,
        maximum_columns: usize,
        maximum_rows: usize,
    ) -> Option<Self> {
        let (columns, rows) = fit_grid_dimensions(
            source_width,
            source_height,
            maximum_columns.min(GRID_MAX_COLUMNS),
            maximum_rows.min(GRID_MAX_ROWS),
        )?;
        let mut cells = vec![Cell::default(); columns * rows];
        if !convert_rgba8(
            pixels,
            source_width,
            source_height,
            columns,
            rows,
            &mut cells,
        ) {
            return None;
        }
        Some(Self {
            cells,
            columns,
            rows,
        })
    }

    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub fn is_populated(&self) -> bool {
        grid_is_populated(self.columns, self.rows)
    }

    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Row-major lookup, `None` outside the grid.
    #[must_use]
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        self.cells.get(row * self.columns + column)
    }

    /// Explicitly discards the grid (`ascii_art_grid_clear`).
    ///
    /// Returns `false` and changes nothing when the grid is already empty, which
    /// is C's rejection of an unpopulated clear.
    pub fn clear(&mut self) -> bool {
        if !self.is_populated() {
            return false;
        }
        self.cells.clear();
        self.columns = 0;
        self.rows = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    //! The fourteen tests from `../musializer/tests/test_ascii_art.c`, ported
    //! with their expectations intact.

    use super::*;

    fn set_gray(pixels: &mut [u8], width: usize, x: usize, y: usize, value: u8) {
        let offset = (y * width + x) * 4;
        pixels[offset] = value;
        pixels[offset + 1] = value;
        pixels[offset + 2] = value;
        pixels[offset + 3] = 255;
    }

    fn convert_one(pixels: &[u8], width: usize, height: usize) -> Cell {
        let mut cell = [Cell::default(); 1];
        assert!(convert_rgba8(pixels, width, height, 1, 1, &mut cell));
        cell[0]
    }

    #[test]
    fn fits_landscape_portrait_and_small_images_without_distortion() {
        assert_eq!(fit_grid_dimensions(1408, 768, 96, 54), Some((96, 52)));
        assert_eq!(fit_grid_dimensions(768, 1408, 96, 54), Some((29, 54)));
        assert_eq!(fit_grid_dimensions(40, 20, 96, 54), Some((40, 20)));
    }

    #[test]
    fn grid_fit_and_layout_are_atomic_on_invalid_input() {
        assert_eq!(fit_grid_dimensions(0, 100, 96, 54), None);
        assert_eq!(grid_layout(f32::NAN, 360.0, 96, 52, 1.0), None);
    }

    #[test]
    fn grid_layout_centers_and_scales_square_and_legacy_cells() {
        let square = grid_layout(640.0, 360.0, 96, 52, 1.0).expect("a valid square layout");
        assert!((square.cell_width - 640.0 / 96.0).abs() < 0.0001);
        assert!((square.cell_height - square.cell_width).abs() < 0.0001);
        assert!((square.field_width - 640.0).abs() < 0.0001);
        assert!(square.offset_y > 0.0);

        let doubled = grid_layout(1280.0, 720.0, 96, 52, 1.0).expect("a valid doubled layout");
        assert!((doubled.cell_width - square.cell_width * 2.0).abs() < 0.0001);
        assert!((doubled.field_height - square.field_height * 2.0).abs() < 0.0001);

        let legacy = grid_layout(640.0, 360.0, 96, 26, 0.5).expect("a valid legacy layout");
        assert!(
            (legacy.field_width / legacy.field_height - 96.0 * 0.5 / 26.0).abs() < 0.0001,
            "a half-height grid keeps its display aspect"
        );
    }

    #[test]
    fn maps_flat_black_and_white_to_tone_extremes() {
        let cell = convert_one(&[0, 0, 0, 255], 1, 1);
        assert_eq!(cell.glyph, u32::from(b' '));
        assert_eq!(cell.luminance, 0.0);
        assert_eq!(cell.edge_strength, 0.0);
        assert_eq!(cell.edge_orientation, EdgeOrientation::None);

        let cell = convert_one(&[255, 255, 255, 255], 1, 1);
        assert_eq!(cell.glyph, u32::from(b'@'));
        assert_eq!(cell.luminance, 1.0);
        assert_eq!(cell.edge_strength, 0.0);
        assert_eq!(cell.edge_orientation, EdgeOrientation::None);
    }

    #[test]
    fn detects_vertical_and_horizontal_edges() {
        let mut vertical = [0u8; 5 * 5 * 4];
        let mut horizontal = [0u8; 5 * 5 * 4];
        for y in 0..5 {
            for x in 0..5 {
                set_gray(&mut vertical, 5, x, y, if x >= 2 { 255 } else { 0 });
                set_gray(&mut horizontal, 5, x, y, if y >= 2 { 255 } else { 0 });
            }
        }

        let cell = convert_one(&vertical, 5, 5);
        assert_eq!(cell.edge_orientation, EdgeOrientation::Vertical);
        assert_eq!(cell.glyph, u32::from(b'|'));
        assert!(cell.edge_strength >= 0.05);

        let cell = convert_one(&horizontal, 5, 5);
        assert_eq!(cell.edge_orientation, EdgeOrientation::Horizontal);
        assert_eq!(cell.glyph, u32::from(b'-'));
        assert!(cell.edge_strength >= 0.05);
    }

    #[test]
    fn detects_both_diagonal_edge_directions() {
        let mut forward = [0u8; 7 * 7 * 4];
        let mut back = [0u8; 7 * 7 * 4];
        for y in 0..7 {
            for x in 0..7 {
                set_gray(&mut forward, 7, x, y, if x + y >= 6 { 255 } else { 0 });
                set_gray(&mut back, 7, x, y, if x >= y { 255 } else { 0 });
            }
        }

        let cell = convert_one(&forward, 7, 7);
        assert_eq!(cell.edge_orientation, EdgeOrientation::DiagonalForward);
        assert_eq!(cell.glyph, u32::from(b'/'));

        let cell = convert_one(&back, 7, 7);
        assert_eq!(cell.edge_orientation, EdgeOrientation::DiagonalBack);
        assert_eq!(cell.glyph, u32::from(b'\\'));
    }

    #[test]
    fn keeps_crossed_detail_as_tone_instead_of_arbitrary_edge() {
        let mut cross = [0u8; 9 * 9 * 4];
        for y in 0..9 {
            for x in 0..9 {
                set_gray(&mut cross, 9, x, y, if x == 4 || y == 4 { 255 } else { 0 });
            }
        }
        let cell = convert_one(&cross, 9, 9);
        assert_eq!(cell.edge_orientation, EdgeOrientation::None);
        for glyph in [b'-', b'|', b'/', b'\\'] {
            assert_ne!(cell.glyph, u32::from(glyph));
        }
    }

    #[test]
    fn averages_cell_color_and_alpha() {
        #[rustfmt::skip]
        let pixels = [
            10, 20, 30, 40,   30, 40, 50, 60,
            50, 60, 70, 80,   70, 80, 90, 100u8,
        ];
        let cell = convert_one(&pixels, 2, 2);
        assert_eq!(
            cell.foreground,
            Rgba {
                r: 47,
                g: 57,
                b: 67,
                a: 70
            }
        );
    }

    #[test]
    fn ignores_fully_transparent_rgb_when_averaging_color() {
        #[rustfmt::skip]
        let pixels = [
            255, 0, 0, 0,
            0, 20, 240, 255u8,
        ];
        let cell = convert_one(&pixels, 2, 1);
        assert_eq!(
            cell.foreground,
            Rgba {
                r: 0,
                g: 20,
                b: 240,
                a: 128
            }
        );
    }

    #[test]
    fn uses_robust_contrast_to_retain_narrow_range_detail() {
        let mut pixels = [0u8; 64 * 4];
        for x in 0..64 {
            set_gray(&mut pixels, 64, x, 0, (40 + x) as u8);
        }
        let mut cells = [Cell::default(); 4];
        assert!(convert_rgba8(&pixels, 64, 1, 4, 1, &mut cells));
        assert!(cells[0].luminance < 0.35);
        assert!(cells[3].luminance > 0.80);
        assert_ne!(cells[0].glyph, cells[3].glyph);
    }

    #[test]
    fn compensates_for_sparse_glyph_ink_in_dark_source_detail() {
        let cell = convert_one(&[32, 32, 32, 255], 1, 1);
        assert!(cell.luminance > 0.18);
        assert!(cell.luminance < 0.35);
        assert_ne!(cell.glyph, u32::from(b' '));
    }

    #[test]
    fn rejects_invalid_input_without_touching_output() {
        let pixel = [0u8, 0, 0, 255];
        let sentinel = Cell {
            glyph: 0x1234_5678,
            ..Cell::default()
        };
        let mut output = [sentinel; 2];
        // Zero source width, a grid wider than the source, and no output room.
        assert!(!convert_rgba8(&pixel, 0, 1, 1, 1, &mut output));
        assert!(!convert_rgba8(&pixel, 1, 1, 2, 1, &mut output));
        assert!(!convert_rgba8(&pixel, 1, 1, 1, 1, &mut output[..0]));
        // Rust also rejects a source buffer shorter than the dimensions claim,
        // where C would read past the end.
        assert!(!convert_rgba8(&pixel[..3], 1, 1, 1, 1, &mut output));
        assert_eq!(output[0].glyph, 0x1234_5678);
        assert_eq!(output[1].glyph, 0x1234_5678);
    }

    #[test]
    fn conversion_is_byte_reproducible() {
        let mut pixels = [0u8; 8 * 6 * 4];
        for (i, byte) in pixels.iter_mut().enumerate() {
            *byte = ((i * 73 + 19) % 256) as u8;
        }
        let mut first = [Cell::default(); 12];
        let mut second = [Cell::default(); 12];
        assert!(convert_rgba8(&pixels, 8, 6, 4, 3, &mut first));
        assert!(convert_rgba8(&pixels, 8, 6, 4, 3, &mut second));
        assert_eq!(first, second);
    }

    #[test]
    fn animation_cycles_tone_glyphs_but_preserves_image_structure() {
        let tone = Cell {
            glyph: u32::from(b'L'),
            luminance: 0.58,
            edge_orientation: EdgeOrientation::None,
            ..Cell::default()
        };
        let first = animated_glyph(&tone, 7, 11, 0.0, 1.0, 0x1234);
        let mut changed = false;
        for frame in 1..80u32 {
            let glyph = animated_glyph(&tone, 7, 11, f64::from(frame) / 12.0, 1.0, 0x1234);
            assert!(
                TONE_RAMP.contains(&(glyph as u8)),
                "glyph {glyph} left the ramp"
            );
            if glyph != first {
                changed = true;
            }
        }
        assert!(changed, "a lit tone cell must animate");

        // A coherent edge never animates: that is what keeps a silhouette.
        let edge = Cell {
            glyph: u32::from(b'/'),
            edge_orientation: EdgeOrientation::DiagonalForward,
            ..tone
        };
        assert_eq!(
            animated_glyph(&edge, 7, 11, 99.0, 1.0, 0x1234),
            u32::from(b'/')
        );

        let empty = Cell {
            glyph: u32::from(b' '),
            luminance: 0.0,
            ..tone
        };
        assert_eq!(
            animated_glyph(&empty, 7, 11, 99.0, 1.0, 0x1234),
            u32::from(b' ')
        );
    }

    #[test]
    fn animation_is_seek_deterministic_and_sanitizes_inputs() {
        let cell = Cell {
            glyph: u32::from(b'G'),
            luminance: 0.72,
            edge_orientation: EdgeOrientation::None,
            ..Cell::default()
        };
        let seeked = animated_glyph(&cell, 19, 31, 47.125, 0.83, 0xfeed_beef);
        assert_eq!(
            animated_glyph(&cell, 19, 31, 47.125, 0.83, 0xfeed_beef),
            seeked,
            "the same playhead gives the same glyph"
        );
        assert_eq!(
            animated_glyph(&cell, 19, 31, f64::NAN, f32::NAN, 0xfeed_beef),
            animated_glyph(&cell, 19, 31, 0.0, 0.0, 0xfeed_beef),
            "non-finite inputs sanitize to zero"
        );
    }

    #[test]
    fn a_grid_clears_explicitly_and_only_once() {
        let mut pixels = [0u8; 8 * 6 * 4];
        for (i, byte) in pixels.iter_mut().enumerate() {
            *byte = ((i * 31 + 7) % 256) as u8;
        }
        let mut grid = Grid::from_rgba8(&pixels, 8, 6, 96, 54).expect("a valid grid");
        assert_eq!((grid.columns(), grid.rows()), (8, 6));
        assert!(grid.is_populated());
        assert!(grid.cell(5, 7).is_some());
        assert!(grid.cell(6, 0).is_none());
        assert!(grid.clear());
        assert!(!grid.is_populated());
        assert_eq!(grid.cells().len(), 0);
        // A second clear is C's rejected clear of an unpopulated grid.
        assert!(!grid.clear());
        assert_eq!(grid, Grid::default());
    }

    #[test]
    fn a_grid_bounds_itself_to_the_c_maximums() {
        // A 4K frame must still fit the fixed 96x54 buffer.
        let pixels = vec![128u8; 3840 * 4];
        let grid = Grid::from_rgba8(&pixels, 3840, 1, usize::MAX, usize::MAX)
            .expect("a wide image still fits");
        assert!(grid.columns() <= GRID_MAX_COLUMNS);
        assert!(grid.rows() <= GRID_MAX_ROWS);
        assert!(grid.cells().len() <= GRID_MAX_CELLS);
    }
}
