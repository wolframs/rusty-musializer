//! Resolving a supersampled frame down to the output size, in linear light.
//!
//! # Why this exists, and why it is not `Image::resize`
//!
//! High and Master render into a target `supersample_factor` times the output on
//! both axes and then downsample. That downsample used to be raylib's
//! `ImageResize`, which for an 8-bit RGBA image takes a fast path calling
//! **`stbir_resize_uint8_linear`** (`vendor/raylib-6.0/src/rtextures.c:1759-1762`).
//! `_linear` in stb's naming means "the data *is* linear light" — the sibling
//! that actually decodes sRGB first is `stbir_resize_uint8_srgb`
//! (`stb_image_resize2.h:8009`). Our frames are sRGB-encoded 8-bit, so that call
//! averaged gamma-encoded code values as though they were light.
//!
//! Averaging in gamma space systematically **darkens** every high-contrast edge,
//! because the transfer function is convex: the mean of the encoded values is
//! below the encoding of the mean. For a music visualiser — thin bright lines,
//! particles and glyph strokes on a near-black field — that is the whole
//! picture. Measured on a one-output-pixel white line against the app's own
//! `0x151515` background at 2x, integrating the linear light across the profile:
//!
//! | downsample | resulting profile | integrated light |
//! | --- | --- | --- |
//! | none (a 1x render) | `255` | 0.9925 |
//! | gamma-space Mitchell (what shipped) | `22 51 205 51 22` | **0.6618** |
//! | linear-light, this module | — | 0.9925 |
//!
//! So **Master rendered thin bright detail measurably worse than Balanced**,
//! which renders at 1x and never resamples at all. That is the operator's "I
//! can't claim that the export quality is particularly high", as a number.
//!
//! # Why a box average rather than Mitchell
//!
//! The factor is always an exact integer (1 or 2 — `RenderExportConfig::validate`
//! refuses anything else), so every output pixel is backed by exactly
//! `factor * factor` input pixels with no partial coverage and no resampling
//! kernel to choose. A box average over those is the *definition* of the light
//! that fell on the output pixel. Mitchell's negative lobes would ring a hard
//! edge into `6 99 229 99 6` even done correctly — softer than the 1x render's
//! `255`, for no gain, since there is no aliasing left to suppress once the
//! source is already at the sample rate the kernel would be reconstructing.
//!
//! # Alpha
//!
//! Averaged linearly, without a transfer function, because alpha is a coverage
//! fraction and was never gamma-encoded. `stbir`'s `STBIR_RGBA` layout instead
//! premultiplies, filters and un-premultiplies, which biases RGB by up to 23/255
//! next to anything translucent — for nothing, since swscale discards alpha on
//! the way to `yuv420p`.
//!
//! # A 16-bit source, and the dither (EX3)
//!
//! [`LinearResolver::resolve_deep`] takes the export's 16-bit target
//! (`runtime::deep_target`) and is where the frame is quantized to 8 bits for
//! the last time. A slow dark gradient rounded there without a dither is a
//! staircase of one-code steps, and one code near black is a visible contour;
//! the same gradient with a dither is the same mean light with the step
//! dissolved into noise below a code. So every value that is *between* two
//! codes is dithered, and every value that *is* a code passes through
//! untouched — a flat fill, a palette colour, anything the 8-bit render would
//! have produced exactly, comes out exactly.
//!
//! The dither is rectangular: `floor(code + u)` with `u` uniform in
//! `0.0..1.0`, which keeps the mean light exactly and adds at most one code.
//! Triangular dither (two uniforms, ±1 code) is the textbook alternative and
//! was measured against it on Constellation's vignetted corner: the same
//! banding left in the decoded MP4 (0.077 against 0.076 codes at High), for
//! 16 % more bytes. The noise is a fixed integer hash of the pixel's position
//! and channel — no wall clock, no frame counter — so a project renders the
//! same frames on every run, a still is its video frame, and a picture that
//! holds still holds still in the noise too, which is also what x264's inter
//! prediction keeps best: on a 0→40 ramp at High, a static dither leaves a
//! 0.68-code staircase in the decoded file where one reseeded every frame
//! leaves 0.91.

/// How many bins the encode table splits linear `0.0..=1.0` into.
///
/// 8192 puts roughly five bins inside the *first* 8-bit code step (sRGB code 1
/// is 0.000303 in linear light, and a bin is 0.000122 wide), which is where the
/// transfer function is steepest and where a coarse table would visibly quantise
/// the shadows this application is almost entirely made of.
const ENCODE_BINS: usize = 8192;

/// Why a frame could not be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The source buffer is not `width * height * 4` bytes.
    #[error("the supersampled frame is not the size its dimensions claim")]
    SourceLength,
    /// The source is not an exact integer multiple of the destination on both
    /// axes, by the same factor.
    #[error("the supersampled frame is not a uniform integer multiple of the output")]
    Geometry,
}

/// sRGB code value to linear light, the IEC 61966-2-1 electro-optical transfer
/// function.
#[must_use]
fn srgb_to_linear(code: u8) -> f32 {
    let value = f32::from(code) / 255.0;
    if value <= 0.040_448_237 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse: linear light to an sRGB code value in `0.0..=1.0`.
#[must_use]
fn linear_to_srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// The two lookup tables a resolve needs, built once and reused per frame.
///
/// Built rather than `const`, because `powf` is not available in a `const fn`.
/// Owned by the caller rather than kept in a `static`, because `musializer-core`
/// holds no global mutable state and a table that is rebuilt per export is
/// 8 KiB and microseconds against a whole encode.
#[derive(Clone, Debug)]
pub struct LinearResolver {
    decode: [f32; 256],
    encode: Vec<u8>,
    /// A 16-bit sRGB value (`0..=65535`) to linear light.
    decode_deep: Vec<f32>,
    /// Linear light at `ENCODE_BINS` even steps to the *continuous* sRGB code
    /// value (`0.0..=255.0`), interpolated between: the dither needs the
    /// fraction the nearest-code table throws away.
    encode_fine: Vec<f32>,
}

impl Default for LinearResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl LinearResolver {
    #[must_use]
    pub fn new() -> Self {
        let mut decode = [0.0f32; 256];
        for (code, slot) in decode.iter_mut().enumerate() {
            *slot = srgb_to_linear(code as u8);
        }
        // Each bin stores the *nearest* code rather than the truncation of the
        // analytic inverse, so the table cannot systematically darken the way
        // the bug this module replaces did. `+ 0.5` rounds; the clamp is belt
        // and braces against the last bin's float landing a hair over 1.0.
        let mut encode = vec![0u8; ENCODE_BINS];
        for (bin, slot) in encode.iter_mut().enumerate() {
            let linear = bin as f32 / (ENCODE_BINS - 1) as f32;
            let coded = linear_to_srgb(linear) * 255.0;
            *slot = coded.clamp(0.0, 255.0).round() as u8;
        }
        let decode_deep = (0..=u16::MAX)
            .map(|value| {
                let coded = f32::from(value) / 65535.0;
                if coded <= 0.040_448_237 {
                    coded / 12.92
                } else {
                    ((coded + 0.055) / 1.055).powf(2.4)
                }
            })
            .collect();
        let encode_fine = (0..=ENCODE_BINS)
            .map(|bin| linear_to_srgb(bin as f32 / ENCODE_BINS as f32).clamp(0.0, 1.0) * 255.0)
            .collect();
        Self {
            decode,
            encode,
            decode_deep,
            encode_fine,
        }
    }

    /// Linear light to the continuous sRGB code value, by the fine table.
    fn encode_continuous(&self, linear: f32) -> f32 {
        let position = linear.clamp(0.0, 1.0) * ENCODE_BINS as f32;
        let bin = (position as usize).min(ENCODE_BINS - 1);
        let fraction = position - bin as f32;
        self.encode_fine[bin] + (self.encode_fine[bin + 1] - self.encode_fine[bin]) * fraction
    }

    /// Resolves a **16-bit** RGBA source (the export's deep target, EX3) into
    /// 8-bit `destination`, averaging in linear light when `source` is a
    /// supersampled multiple, and dithering every value that falls between
    /// two 8-bit codes (see the module notes).
    ///
    /// A value the 8-bit render would have produced exactly (`code * 257`, or
    /// a block of identical such values) comes out as that code, bit for bit.
    ///
    /// # Errors
    /// As [`resolve`](LinearResolver::resolve).
    pub fn resolve_deep(
        &self,
        source: &[u16],
        source_width: usize,
        source_height: usize,
        width: usize,
        height: usize,
        destination: &mut Vec<u8>,
    ) -> Result<(), ResolveError> {
        let expected = source_width
            .checked_mul(source_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ResolveError::SourceLength)?;
        if source.len() != expected {
            return Err(ResolveError::SourceLength);
        }
        if width == 0 || height == 0 || source_width == 0 || source_height == 0 {
            return Err(ResolveError::Geometry);
        }
        if !source_width.is_multiple_of(width) || !source_height.is_multiple_of(height) {
            return Err(ResolveError::Geometry);
        }
        let factor = source_width / width;
        if factor != source_height / height {
            return Err(ResolveError::Geometry);
        }

        destination.clear();
        destination.reserve(width * height * 4);
        let samples = (factor * factor) as f32;
        for row in 0..height {
            for column in 0..width {
                let noise = Dither::at(column as u32, row as u32);
                for channel in 0..3 {
                    let first =
                        source[(row * factor * source_width + column * factor) * 4 + channel];
                    let mut uniform = true;
                    let mut light = 0.0f32;
                    for sub_row in 0..factor {
                        let base = ((row * factor + sub_row) * source_width + column * factor) * 4;
                        for sub_column in 0..factor {
                            let value = source[base + sub_column * 4 + channel];
                            uniform &= value == first;
                            light += self.decode_deep[value as usize];
                        }
                    }
                    let code = if uniform && first.is_multiple_of(257) {
                        // A code the 8-bit render would have written: exact.
                        (first / 257) as u8
                    } else if uniform {
                        noise.quantize(channel, f32::from(first) / 257.0)
                    } else {
                        noise.quantize(channel, self.encode_continuous(light / samples))
                    };
                    destination.push(code);
                }
                // Coverage, not light: no transfer function and no dither.
                let mut alpha = 0u32;
                for sub_row in 0..factor {
                    let base = ((row * factor + sub_row) * source_width + column * factor) * 4;
                    for sub_column in 0..factor {
                        alpha += u32::from(source[base + sub_column * 4 + 3]);
                    }
                }
                destination.push((alpha as f32 / samples / 257.0).clamp(0.0, 255.0).round() as u8);
            }
        }
        Ok(())
    }

    /// Resolves `source` (RGBA8, `source_width` x `source_height`) into
    /// `destination` at `width` x `height`, averaging in linear light.
    ///
    /// `destination` is cleared and refilled, so one `Vec` can be reused for a
    /// whole export without reallocating after the first frame.
    ///
    /// A factor of 1 is a straight copy with **no colour conversion at all**.
    /// That is not an optimization: Balanced must stay byte-identical to what it
    /// produced before this module existed, and a round trip through the two
    /// tables would move a handful of codes for no reason.
    ///
    /// # Errors
    /// [`ResolveError::SourceLength`] when `source` is not
    /// `source_width * source_height * 4` bytes; [`ResolveError::Geometry`] for a
    /// zero dimension, a non-multiple, or a non-uniform factor.
    pub fn resolve(
        &self,
        source: &[u8],
        source_width: usize,
        source_height: usize,
        width: usize,
        height: usize,
        destination: &mut Vec<u8>,
    ) -> Result<(), ResolveError> {
        let expected = source_width
            .checked_mul(source_height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ResolveError::SourceLength)?;
        if source.len() != expected {
            return Err(ResolveError::SourceLength);
        }
        if width == 0 || height == 0 || source_width == 0 || source_height == 0 {
            return Err(ResolveError::Geometry);
        }
        if source_width % width != 0 || source_height % height != 0 {
            return Err(ResolveError::Geometry);
        }
        let factor = source_width / width;
        if factor != source_height / height {
            return Err(ResolveError::Geometry);
        }

        destination.clear();
        destination.reserve(width * height * 4);
        if factor == 1 {
            destination.extend_from_slice(source);
            return Ok(());
        }

        let samples = (factor * factor) as f32;
        let scale = (ENCODE_BINS - 1) as f32;
        for row in 0..height {
            for column in 0..width {
                let (mut red, mut green, mut blue, mut alpha) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                for sub_row in 0..factor {
                    let base = ((row * factor + sub_row) * source_width + column * factor) * 4;
                    for sub_column in 0..factor {
                        let pixel = base + sub_column * 4;
                        red += self.decode[source[pixel] as usize];
                        green += self.decode[source[pixel + 1] as usize];
                        blue += self.decode[source[pixel + 2] as usize];
                        // Coverage, not light: no transfer function.
                        alpha += f32::from(source[pixel + 3]);
                    }
                }
                let encode = |sum: f32| -> u8 {
                    let bin = ((sum / samples) * scale).clamp(0.0, scale) as usize;
                    self.encode[bin]
                };
                destination.push(encode(red));
                destination.push(encode(green));
                destination.push(encode(blue));
                destination.push((alpha / samples).clamp(0.0, 255.0).round() as u8);
            }
        }
        Ok(())
    }
}

/// One pixel's dither: three 16-bit uniforms from one 64-bit integer hash of
/// its position, one per colour channel.
///
/// Integer arithmetic only (splitmix64's finalizer), for the reason the shader
/// hashes are integer (`AGENTS.md`): the same input is the same noise on every
/// machine and every run.
#[derive(Clone, Copy, Debug)]
struct Dither(u64);

impl Dither {
    fn at(column: u32, row: u32) -> Self {
        let mut z = (u64::from(row) << 32 | u64::from(column)).wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Dither(z ^ (z >> 31))
    }

    /// Uniform noise in `0.0..1.0` for `channel`.
    fn uniform(self, channel: usize) -> f32 {
        ((self.0 >> (channel * 16)) & 0xFFFF) as f32 / 65536.0
    }

    /// `code` (continuous, `0.0..=255.0`) to the 8-bit code the dither picks:
    /// the code below with the probability of the distance to the code above,
    /// so the mean is `code` exactly.
    fn quantize(self, channel: usize, code: f32) -> u8 {
        (code + self.uniform(channel)).floor().clamp(0.0, 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8-bit codes widened to the deep target's storage, as GL stores an 8-bit
    /// colour in `GL_RGBA16`.
    fn widen(source: &[u8]) -> Vec<u16> {
        source.iter().map(|&code| u16::from(code) * 257).collect()
    }

    /// Anything the 8-bit render would have produced exactly comes out
    /// exactly, at both factors: a flat fill, a palette colour, a hard edge
    /// between two codes at 1x.
    #[test]
    fn exact_codes_pass_through_a_deep_resolve_untouched() {
        let resolver = LinearResolver::new();
        let source: Vec<u8> = (0..(16 * 8 * 4))
            .map(|byte| (byte * 37 % 256) as u8)
            .collect();
        let mut destination = Vec::new();
        resolver
            .resolve_deep(&widen(&source), 16, 8, 16, 8, &mut destination)
            .expect("1x");
        assert_eq!(destination, source);
        for code in [0u8, 1, 21, 128, 254, 255] {
            let flat = vec![code; 8 * 8 * 4];
            resolver
                .resolve_deep(&widen(&flat), 8, 8, 4, 4, &mut destination)
                .expect("2x");
            assert_eq!(destination, vec![code; 4 * 4 * 4], "flat {code} drifted");
        }
    }

    /// The 2x deep resolve is the 8-bit resolve plus at most one code of
    /// dither: the same linear-light average, read more finely.
    #[test]
    fn a_deep_box_average_agrees_with_the_eight_bit_one_within_the_dither() {
        let resolver = LinearResolver::new();
        let source: Vec<u8> = (0..(32 * 32 * 4))
            .map(|byte| (byte * 7919 % 251) as u8)
            .collect();
        let (mut eight, mut deep) = (Vec::new(), Vec::new());
        resolver
            .resolve(&source, 32, 32, 16, 16, &mut eight)
            .expect("8-bit");
        resolver
            .resolve_deep(&widen(&source), 32, 32, 16, 16, &mut deep)
            .expect("deep");
        for (index, (a, b)) in eight.iter().zip(&deep).enumerate() {
            assert!(a.abs_diff(*b) <= 1, "byte {index}: 8-bit {a}, deep {b}");
        }
    }

    /// The measurement this exists for. A 0→40 ramp across 1920 columns in
    /// 16-bit, quantized two ways: rounded, its column means form a staircase
    /// half a code high, the contour a viewer sees; dithered, the staircase is
    /// gone into noise and the mean light is unchanged.
    ///
    /// Negative control (2026-11-13): `quantize` rounding instead of
    /// dithering failed the staircase assertion; reverted byte for byte.
    #[test]
    fn a_dithered_ramp_loses_its_staircase_and_keeps_its_light() {
        let resolver = LinearResolver::new();
        let (width, height) = (1920usize, 64usize);
        let mut source = Vec::with_capacity(width * height * 4);
        for _ in 0..height {
            for column in 0..width {
                let code = 40.0 * column as f32 / (width - 1) as f32;
                let value = (code * 257.0).round() as u16;
                source.extend_from_slice(&[value, value, value, 65535]);
            }
        }
        let mut destination = Vec::new();
        resolver
            .resolve_deep(&source, width, height, width, height, &mut destination)
            .expect("1x");
        // Column means over every row, against the ideal code.
        let staircase = |pixels: &dyn Fn(usize, usize) -> f32| -> f32 {
            (0..width)
                .map(|column| {
                    let mean =
                        (0..height).map(|row| pixels(row, column)).sum::<f32>() / height as f32;
                    (mean - 40.0 * column as f32 / (width - 1) as f32).abs()
                })
                .fold(0.0, f32::max)
        };
        let dithered = staircase(&|row, column| f32::from(destination[(row * width + column) * 4]));
        let rounded = staircase(&|row, column| {
            (f32::from(source[(row * width + column) * 4]) / 257.0).round()
        });
        assert!(rounded > 0.45, "the rounded ramp is a staircase: {rounded}");
        assert!(
            dithered < 0.25,
            "the dithered ramp should have no staircase left: {dithered}"
        );
        // Deterministic: the same frame twice is the same bytes.
        let mut again = Vec::new();
        resolver
            .resolve_deep(&source, width, height, width, height, &mut again)
            .expect("1x");
        assert_eq!(again, destination);
    }

    /// The dither's noise is uniform on `0.0..1.0`: bounded, and centred on
    /// one half, so `floor(code + u)` keeps the mean.
    #[test]
    fn the_dither_is_uniform_and_centred() {
        let mut sum = 0.0f64;
        let count = 256 * 256;
        for row in 0..256 {
            for column in 0..256 {
                let noise = Dither::at(column, row);
                for channel in 0..3 {
                    let value = noise.uniform(channel);
                    assert!((0.0..1.0).contains(&value), "{value}");
                    sum += f64::from(value);
                }
            }
        }
        let mean = sum / f64::from(count * 3);
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
    }

    /// The two tables have to be inverses of each other to within a code value,
    /// or a resolve would shift a flat colour that has nothing to average.
    #[test]
    fn the_tables_round_trip_every_code_value_exactly() {
        let resolver = LinearResolver::new();
        let scale = (ENCODE_BINS - 1) as f32;
        for code in 0..=255u8 {
            let linear = resolver.decode[code as usize];
            let bin = (linear * scale).clamp(0.0, scale) as usize;
            assert_eq!(
                resolver.encode[bin], code,
                "code {code} came back as {} (linear {linear})",
                resolver.encode[bin]
            );
        }
    }

    /// A flat field must survive untouched. This is the property a gamma-space
    /// average also satisfies, which is exactly why the bug was invisible in
    /// every smooth region and only showed at edges.
    #[test]
    fn a_flat_field_resolves_to_itself() {
        let resolver = LinearResolver::new();
        for code in [0u8, 1, 17, 21, 128, 200, 254, 255] {
            let source = vec![code; 4 * 4 * 4];
            let mut destination = Vec::new();
            resolver
                .resolve(&source, 4, 4, 2, 2, &mut destination)
                .expect("2x2 from 4x4");
            assert_eq!(destination, vec![code; 2 * 2 * 4], "flat {code} drifted");
        }
    }

    /// The measurement this module exists for.
    ///
    /// One white sample in a 2x2 block against the application's own background
    /// clear (`0x15` = 21). The right answer preserves the *light*: a quarter of
    /// white plus three quarters of background. A gamma-space average — what
    /// `stbir_resize_uint8_linear` does — returns the mean of the code values,
    /// `(255 + 21 * 3) / 4 = 79`, which is barely half the light.
    #[test]
    fn a_bright_sample_keeps_its_light_rather_than_its_code_value() {
        let resolver = LinearResolver::new();
        let background = 21u8;
        let mut source = vec![background; 2 * 2 * 4];
        source[0] = 255;
        source[1] = 255;
        source[2] = 255;
        for byte in source.iter_mut().skip(3).step_by(4) {
            *byte = 255;
        }
        let mut destination = Vec::new();
        resolver
            .resolve(&source, 2, 2, 1, 1, &mut destination)
            .expect("1x1 from 2x2");

        let expected_light = (srgb_to_linear(255) + 3.0 * srgb_to_linear(background)) / 4.0;
        // The tolerance is one 8-bit code step *at this brightness*, not an
        // absolute epsilon: sRGB codes are far apart in the midtones and close
        // together near black, so a fixed epsilon would be slack in the shadows
        // and impossible to meet at code 138.
        let ideal = linear_to_srgb(expected_light) * 255.0;
        assert!(
            (f32::from(destination[0]) - ideal).abs() <= 1.0,
            "resolved to code {} (light {}), wanted {ideal:.2} (light {expected_light})",
            destination[0],
            srgb_to_linear(destination[0])
        );

        // And it is emphatically not the gamma-space answer, which is what the
        // shipped path returned. Both numbers are pinned so a "simplification"
        // back to a code-value average fails here rather than in an export
        // nobody measures.
        let gamma_space = (255u32 + u32::from(background) * 3) / 4;
        assert_eq!(gamma_space, 79);
        assert_eq!(
            destination[0], 138,
            "the linear-light answer for one white sample in four over 0x15"
        );
    }

    /// A factor of 1 must be a byte-for-byte copy: Balanced never supersamples,
    /// and its output has to be unchanged by this module existing.
    #[test]
    fn a_factor_of_one_is_a_copy_and_not_a_round_trip() {
        let resolver = LinearResolver::new();
        let source: Vec<u8> = (0..(3 * 2 * 4)).map(|byte| byte as u8).collect();
        let mut destination = Vec::new();
        resolver
            .resolve(&source, 3, 2, 3, 2, &mut destination)
            .expect("identity");
        assert_eq!(destination, source);
    }

    #[test]
    fn degenerate_geometry_is_refused_rather_than_guessed_at() {
        let resolver = LinearResolver::new();
        let source = vec![0u8; 4 * 4 * 4];
        let mut destination = Vec::new();
        // Short buffer.
        assert_eq!(
            resolver.resolve(&source[..8], 4, 4, 2, 2, &mut destination),
            Err(ResolveError::SourceLength)
        );
        // Non-multiple.
        assert_eq!(
            resolver.resolve(&source, 4, 4, 3, 2, &mut destination),
            Err(ResolveError::Geometry)
        );
        // Non-uniform: 2x horizontally, 1x vertically.
        assert_eq!(
            resolver.resolve(&source, 4, 4, 2, 4, &mut destination),
            Err(ResolveError::Geometry)
        );
        // Zero.
        assert_eq!(
            resolver.resolve(&source, 4, 4, 0, 2, &mut destination),
            Err(ResolveError::Geometry)
        );
    }

    /// Every output pixel reads its own 2x2 block and no other. A transposed
    /// index would still produce a plausible picture — mirrored, or sheared by a
    /// row — which no capture would flag, so the mapping is pinned directly.
    #[test]
    fn each_output_pixel_averages_its_own_block() {
        let resolver = LinearResolver::new();
        // A 4x2 source: left half black, right half white, alpha opaque.
        let mut source = vec![0u8; 4 * 2 * 4];
        for row in 0..2 {
            for column in 0..4 {
                let pixel = (row * 4 + column) * 4;
                let value = if column >= 2 { 255 } else { 0 };
                source[pixel] = value;
                source[pixel + 1] = value;
                source[pixel + 2] = value;
                source[pixel + 3] = 255;
            }
        }
        let mut destination = Vec::new();
        resolver
            .resolve(&source, 4, 2, 2, 1, &mut destination)
            .expect("2x1 from 4x2");
        assert_eq!(destination.len(), 2 * 4);
        assert_eq!(destination[0], 0, "the left output pixel is all black");
        assert_eq!(destination[4], 255, "the right output pixel is all white");
    }
}
