//! Whole-file decoding: audio to PCM, images to RGBA8.
//!
//! Three callers want a whole file in memory rather than a stream, and all three
//! are on the same seam — a raylib loader that allocates, a slice formed from its
//! pointer, and an unload that must happen no matter what the caller does next:
//!
//! - the timeline's waveform envelope (`load_timeline_waveform`, `plug.c:688-709`),
//! - Song Atlas's whole-track terrain (`ensure_song_atlas_map`, `plug.c:712-737`),
//! - the ASCII glyph grid (`load_ascii_image_grid`, `plug.c:860-891`).
//!
//! They live together because the mistake they can each make is the same one, and
//! it has already been made once in this codebase's dependency: **a raylib loader
//! that returns a pointer does not tell you how many elements it allocated.**
//! raylib-rs's `Wave::load_samples` sizes its slice by `frameCount` where
//! `LoadWaveSamples` allocates `frameCount * channels`, so for any stereo track
//! the safe wrapper hands back half the audio with no error. Its `get_image_data`
//! has the same shape and forms its slice without null-checking the pointer at
//! all. Every length here is derived from the format the loader itself reports,
//! and every allocation goes straight back to its matching unload.
//!
//! Nothing here needs a window: raylib's `LoadWave`/`LoadImage` are CPU-side, so
//! all of this is reachable from a headless test.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use raylib::audio::{RaylibAudio, Wave};
use raylib::consts::PixelFormat;
use raylib::texture::Image;

use crate::process::process_group::{
    kill_group_and_reap, wait_bounded, ShutdownPolicy, WaitOutcome,
};

/// A decoded track: interleaved PCM plus the format needed to interpret it.
///
/// `samples.len()` is `frame_count * channels`, which is the invariant the two
/// consumers rely on — [`musializer_core::timing::track_timeline::build_waveform`]
/// and [`musializer_core::audio::song_atlas_map::SongAtlasMap::build`] both take a
/// channel count and divide.
#[derive(Clone, Debug)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
}

impl DecodedAudio {
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.samples.len().checked_div(self.channels).unwrap_or(0)
    }
}

/// Decodes a whole audio file (`LoadWave` + `LoadWaveSamples`, `plug.c:694-701`).
///
/// `None` when the file cannot be decoded, which the C treats as non-fatal at
/// every one of these call sites: it warns and leaves the derived data empty.
#[must_use]
pub fn whole_track(audio: &RaylibAudio, path: &Path) -> Option<DecodedAudio> {
    let path = path.to_str()?;
    let wave = audio.new_wave(path).ok()?;
    if !wave.is_wave_valid() {
        return None;
    }
    Some(DecodedAudio {
        samples: wave_samples(&wave)?,
        channels: wave.channels() as usize,
        sample_rate: wave.sample_rate(),
    })
}

/// Copies a wave's samples out as `frame_count * channels` floats.
///
/// Public because the export path needs the same count from a `Wave` it is
/// already holding for other reasons (it crops and re-exports the same object).
///
/// # The raylib-rs bug this exists for
///
/// `Wave::load_samples` builds its slice as `(pointer, self.frameCount)`
/// (`raylib-6.0.0/src/core/audio.rs:438-443`), but `LoadWaveSamples` allocates
/// and fills `frameCount * channels` floats
/// (`vendor/raylib-6.0/src/raudio.c:1323`). For any stereo track the safe
/// wrapper therefore hands back **exactly half the audio**, silently: an export
/// built on it would analyze the first half of the track spread across the whole
/// timeline, and a Song Atlas built on it would compress the whole terrain into
/// the first half of the track. This is the same class of defect as
/// `load_font_from_memory`'s codepoint miscount, and it gets the same treatment —
/// call the ffi directly and take the length from the format, not from the
/// wrapper.
#[must_use]
pub fn wave_samples(wave: &Wave<'_>) -> Option<Vec<f32>> {
    let count = (wave.frame_count() as usize).checked_mul(wave.channels() as usize)?;
    if count == 0 {
        return Some(Vec::new());
    }
    // SAFETY: `**wave` is a bitwise copy of the `ffi::Wave` this `Wave` owns and
    // is only read; the owner outlives the call, so its `data` pointer is live.
    // `LoadWaveSamples` returns a fresh allocation of exactly
    // `frameCount * channels` floats or null, which is checked before the slice
    // is formed, and the allocation is handed straight back to
    // `UnloadWaveSamples` after the copy — so nothing borrowed here escapes.
    unsafe {
        let pointer = raylib::ffi::LoadWaveSamples(**wave);
        if pointer.is_null() {
            return None;
        }
        let samples = std::slice::from_raw_parts(pointer, count).to_vec();
        raylib::ffi::UnloadWaveSamples(pointer);
        Some(samples)
    }
}

/// An image decoded to tightly packed RGBA8, four bytes per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

/// Why an image could not be turned into RGBA8.
///
/// The C collapses all of these into one `false` (`load_ascii_image_grid` returns
/// `bool`), but the import path here reports to a notice tray rather than to
/// `TraceLog`, and "that file is not an image" and "that image is 0x0" are
/// different things to tell a user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("the path is not valid UTF-8")]
    Path,
    #[error("the file could not be decoded as an image")]
    Decode,
    #[error("the image has no pixels")]
    Empty,
    #[error("the decoded image did not come back as RGBA8")]
    Format,
}

/// Decodes an image file to RGBA8 (`LoadImage` + `ImageFormat`, `plug.c:866-868`).
///
/// The conversion is the oracle's own: `ImageFormat` to `UNCOMPRESSED_R8G8B8A8`
/// and then a read of `image.data`, rather than `LoadImageColors`. The two agree
/// for every format this can plausibly be handed, but only one of them is the
/// function the parity target calls, and a rounding difference in a 16-bit or
/// float source would land in glyph choices that a differential harness compares
/// exactly.
pub fn image_rgba8(path: &Path) -> Result<DecodedImage, ImageError> {
    let path = path.to_str().ok_or(ImageError::Path)?;
    let mut image = Image::load_image(path).map_err(|_| ImageError::Decode)?;
    let (width, height) = (
        image.width().max(0) as usize,
        image.height().max(0) as usize,
    );
    let pixels = image_pixels_rgba8(&mut image)?.to_vec();
    Ok(DecodedImage {
        pixels,
        width,
        height,
    })
}

/// Past this a file is not a still or a contact sheet.
pub const PICTURE_LARGEST_FILE: u64 = 64 * 1024 * 1024;
/// A file this small can claim 16384 × 16384. Refused from the header, before
/// anything allocates. 100 MP holds a contact sheet of 8K stills' worth of
/// tiles; nothing here is drawn larger than the window.
const PICTURE_MOST_PIXELS: u64 = 100_000_000;
/// What one decode may hold at its peak, by [`PictureHeader::working_bytes`].
const PICTURE_MOST_WORKING_BYTES: u64 = 1024 * 1024 * 1024;
/// `open(2)`'s `O_NONBLOCK` on Linux x86-64 and AArch64. Declared by hand like
/// the signals in `process_group`: `libc` is not a dependency of this crate.
const O_NONBLOCK: i32 = 0o4000;
/// How long FFmpeg gets for one still. A healthy decode takes milliseconds.
const FFMPEG_DEADLINE: Duration = Duration::from_secs(15);
const FFMPEG_POLL: Duration = Duration::from_millis(5);
/// How much a read takes between looks at the cancel flag.
const READ_CHUNK: usize = 1024 * 1024;

/// Why a picture is not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureError {
    Missing,
    /// Past the file, pixel or working-memory budget.
    TooLarge,
    Undecodable,
    /// The flag went up: nobody wants the answer.
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PictureFormat {
    /// One row's bytes as the inflater holds them, from bit depth and channels.
    Png {
        row_bits: u64,
    },
    Jpeg,
    WebP,
}

/// What a picture's own first bytes promise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PictureHeader {
    format: PictureFormat,
    width: u32,
    height: u32,
}

impl PictureHeader {
    /// An upper bound on one decode's peak, checked so a claimed size cannot
    /// wrap. Eight bytes a pixel in every case is the RGBA8 the decoder
    /// produces plus `ImageFormat`'s second copy while it converts. PNG adds
    /// the inflated rows, a filter byte each, at 16 bits a channel if it says
    /// so; `stb_image` keeps those beside its output. JPEG adds its planes.
    fn working_bytes(&self) -> Option<u64> {
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        let pixels = width.checked_mul(height)?;
        let rgba = pixels.checked_mul(8)?;
        let extra = match self.format {
            PictureFormat::Png { row_bits } => {
                let row = width.checked_mul(row_bits)?.checked_add(7)? / 8 + 1;
                row.checked_mul(height)?
            }
            PictureFormat::Jpeg => pixels.checked_mul(4)?,
            PictureFormat::WebP => 0,
        };
        rgba.checked_add(extra)
    }

    fn within_budget(&self) -> bool {
        let pixels = u64::from(self.width).checked_mul(u64::from(self.height));
        pixels.is_some_and(|pixels| pixels <= PICTURE_MOST_PIXELS)
            && self
                .working_bytes()
                .is_some_and(|bytes| bytes <= PICTURE_MOST_WORKING_BYTES)
    }
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let pair = bytes.get(at..at + 2)?;
    Some(u32::from(pair[0]) << 8 | u32::from(pair[1]))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let three = bytes.get(at..at + 3)?;
    Some(u32::from(three[0]) | u32::from(three[1]) << 8 | u32::from(three[2]) << 16)
}

fn png_header(bytes: &[u8]) -> Option<PictureHeader> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    let depth = u64::from(*bytes.get(24)?);
    let channels = match *bytes.get(25)? {
        0 | 3 => 1,
        4 => 2,
        2 => 3,
        6 => 4,
        _ => return None,
    };
    Some(PictureHeader {
        format: PictureFormat::Png {
            row_bits: depth * channels,
        },
        width,
        height,
    })
}

/// Walks the segments to the frame header, by their lengths: an EXIF thumbnail
/// holds a whole JPEG, and scanning for a marker would find its size instead.
fn jpeg_header(bytes: &[u8]) -> Option<PictureHeader> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        while *bytes.get(at)? == 0xff {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            // Standalone: no length follows.
            0x01 | 0xd0..=0xd7 => {}
            // Start of scan or end of image with no frame header before it.
            0xd9 | 0xda => return None,
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                return Some(PictureHeader {
                    format: PictureFormat::Jpeg,
                    height: be16(bytes, at + 3)?,
                    width: be16(bytes, at + 5)?,
                });
            }
            _ => at += usize::try_from(be16(bytes, at)?).ok()?,
        }
    }
}

fn webp_header(bytes: &[u8]) -> Option<PictureHeader> {
    let (width, height) = match bytes.get(12..16)? {
        b"VP8X" => (le24(bytes, 24)? + 1, le24(bytes, 27)? + 1),
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let dimension = |at: usize| {
                let pair = bytes.get(at..at + 2)?;
                Some(u32::from(u16::from_le_bytes([pair[0], pair[1]])) & 0x3fff)
            };
            (dimension(26)?, dimension(28)?)
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2f {
                return None;
            }
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        _ => return None,
    };
    Some(PictureHeader {
        format: PictureFormat::WebP,
        width,
        height,
    })
}

/// The size a file claims, from its first bytes, with no decoder involved.
/// Anything this cannot read is refused rather than handed to one, because
/// the budget is only a budget if nothing gets round it.
fn picture_header(bytes: &[u8]) -> Option<PictureHeader> {
    let header = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_header(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg_header(bytes)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp_header(bytes)
    } else {
        None
    }?;
    (header.width > 0 && header.height > 0).then_some(header)
}

/// Reads one open file to the end, through a hard cap. Opening without
/// blocking is what makes a FIFO swapped in under the name an ordinary refusal
/// instead of a thread parked for ever; checking the handle, not the name, is
/// what makes that check about the file that gets read.
fn read_picture(path: &Path, cancel: &AtomicBool) -> Result<Vec<u8>, PictureError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                PictureError::Missing
            }
            _ => PictureError::Undecodable,
        })?;
    let metadata = file.metadata().map_err(|_| PictureError::Undecodable)?;
    if !metadata.is_file() {
        return Err(PictureError::Undecodable);
    }
    if metadata.len() > PICTURE_LARGEST_FILE {
        return Err(PictureError::TooLarge);
    }
    let mut reader = file.take(PICTURE_LARGEST_FILE + 1);
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(PictureError::Cancelled);
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(PictureError::Undecodable),
        }
    }
    // It grew between the check and the read.
    if bytes.len() as u64 > PICTURE_LARGEST_FILE {
        return Err(PictureError::TooLarge);
    }
    Ok(bytes)
}

/// `bytes` through FFmpeg, fed by its stdin so that nothing opens the path a
/// second time. FFmpeg gets its own group, which `process_group` forbids only
/// for the Python helpers, because a kill has to reach whatever it started. Its
/// output is cut at the exact size the header promised, `-max_pixels` stops it
/// allocating for a bitstream that lied, and past `deadline` or the flag it is
/// killed and reaped here.
fn ffmpeg_rgba(
    program: &OsStr,
    bytes: &[u8],
    size: [usize; 2],
    deadline: Duration,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, PictureError> {
    let expected = size[0] * size[1] * 4;
    let mut child = Command::new(program)
        .args(["-nostdin", "-v", "error", "-max_pixels"])
        .arg((size[0] * size[1]).to_string())
        .args(["-f", "webp_pipe", "-i", "pipe:0", "-frames:v", "1"])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| PictureError::Undecodable)?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = kill_group_and_reap(&mut child);
        return Err(PictureError::Undecodable);
    };
    // Two threads, so a full pipe either way cannot wedge the watch below.
    // The scope joins them, and both end once the kill has closed the pipes.
    std::thread::scope(|scope| {
        scope.spawn(move || {
            // EPIPE just means it stopped reading.
            let _ = stdin.write_all(bytes);
        });
        let reader = scope.spawn(move || {
            let mut out = Vec::with_capacity(expected);
            let _ = stdout.take(expected as u64 + 1).read_to_end(&mut out);
            out
        });
        let started = Instant::now();
        let stopped = loop {
            if reader.is_finished() {
                break None;
            }
            if cancel.load(Ordering::Relaxed) {
                break Some(PictureError::Cancelled);
            }
            if started.elapsed() >= deadline {
                break Some(PictureError::Undecodable);
            }
            std::thread::sleep(FFMPEG_POLL);
        };
        if let Some(error) = stopped {
            let _ = kill_group_and_reap(&mut child);
            let _ = reader.join();
            return Err(error);
        }
        let out = reader.join().map_err(|_| PictureError::Undecodable)?;
        // Past the cap it has more to say and nothing will read it.
        let finished = out.len() == expected
            && wait_bounded(&mut child, &ShutdownPolicy::cancel()).is_ok_and(
                |waited| matches!(waited, WaitOutcome::Reaped(status) if status.success()),
            );
        if !finished {
            let _ = kill_group_and_reap(&mut child);
            return Err(PictureError::Undecodable);
        }
        Ok(out)
    })
}

/// Decodes a picture an agent named and hands its RGBA8 to `visit`, by what the
/// file holds rather than what it is called: a `.png` that is a JPEG shows, and
/// a text file called `.png` is refused. The path is opened once; everything
/// after that works from the bytes read, so nothing can be swapped in between.
/// A visitor, so the pixels are never copied out of raylib's allocation.
///
/// Safe off the main thread: `LoadImageFromMemory` is CPU-side and keeps no
/// state between calls. `stb_image` reads PNG and JPEG from the same entry
/// point, which raylib reaches for `.png`. It has no WebP, so that goes through
/// FFmpeg. Once `stb_image` has started, `cancel` waits for it to finish, which
/// the budget bounds.
pub fn with_picture<T>(
    path: &Path,
    cancel: &AtomicBool,
    visit: impl FnOnce(&[u8], [usize; 2]) -> T,
) -> Result<T, PictureError> {
    let bytes = read_picture(path, cancel)?;
    let header = picture_header(&bytes).ok_or(PictureError::Undecodable)?;
    if !header.within_budget() {
        return Err(PictureError::TooLarge);
    }
    let size = [header.width as usize, header.height as usize];
    if header.format == PictureFormat::WebP {
        let pixels = ffmpeg_rgba(OsStr::new("ffmpeg"), &bytes, size, FFMPEG_DEADLINE, cancel)?;
        return Ok(visit(&pixels, size));
    }
    // `load_image_from_mem` unwraps this conversion.
    i32::try_from(bytes.len()).map_err(|_| PictureError::Undecodable)?;
    let mut image =
        Image::load_image_from_mem(".png", &bytes).map_err(|_| PictureError::Undecodable)?;
    let decoded = [
        image.width().max(0) as usize,
        image.height().max(0) as usize,
    ];
    if decoded != size {
        return Err(PictureError::Undecodable);
    }
    let pixels = image_pixels_rgba8(&mut image).map_err(|_| PictureError::Undecodable)?;
    Ok(visit(pixels, size))
}

/// Borrows an already-loaded image's pixels as tightly packed RGBA8.
///
/// The same checks [`image_rgba8`] makes, without the copy: an export reads back
/// a supersampled frame every frame, and at 4K Master that copy is 132 MB of
/// allocation and memcpy per frame for data that is consumed once and immediately.
/// The borrow is tied to the `&mut Image`, so raylib's allocation cannot be
/// unloaded while the slice is alive.
///
/// # Errors
/// [`ImageError::Decode`] for an invalid image or a null pixel pointer,
/// [`ImageError::Empty`] for zero pixels, [`ImageError::Format`] when
/// `ImageFormat` declined the conversion — which it does silently, by leaving
/// the image alone.
pub fn image_pixels_rgba8(image: &mut Image) -> Result<&[u8], ImageError> {
    if !image.is_image_valid() {
        return Err(ImageError::Decode);
    }
    image.set_format(PixelFormat::PIXELFORMAT_UNCOMPRESSED_R8G8B8A8);
    let width = image.width().max(0) as usize;
    let height = image.height().max(0) as usize;
    let expected = width.checked_mul(height).and_then(|p| p.checked_mul(4));
    let Some(expected) = expected.filter(|&bytes| bytes > 0) else {
        return Err(ImageError::Empty);
    };
    // `ImageFormat` is a no-op on a format it does not know how to convert, so
    // this is a real check rather than a restatement of the line above: it is what
    // separates "converted" from "left alone at 4 bytes per pixel by luck".
    if image.get_pixel_data_size() != expected {
        return Err(ImageError::Format);
    }
    let data = image.data.cast_const().cast::<u8>();
    if data.is_null() {
        return Err(ImageError::Decode);
    }
    // SAFETY: the returned slice borrows from `image` for as long as the caller's
    // `&mut` lives, so `Image`'s `Drop` — `UnloadImage` — cannot run while it is
    // alive, and nothing else can obtain a second reference to the allocation.
    // The length is not guessed: `GetPixelDataSize` is the same function raylib
    // itself sizes the buffer with in `ImageFormat`, and it was just checked
    // against `width * height * 4`, so the slice cannot outrun the allocation.
    Ok(unsafe { std::slice::from_raw_parts(data, expected) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch file that takes itself with it.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("musializer-decode-{}-{name}", std::process::id()));
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// The round trip that proves the decode, rather than proving that a call
    /// returned: a known image is generated, exported, decoded back, and every
    /// channel of one known pixel is compared.
    #[test]
    fn a_generated_image_decodes_back_to_its_own_pixels() {
        let scratch = Scratch::new("solid.png");
        let source = Image::gen_image_color(4, 3, raylib::color::Color::new(10, 20, 30, 255));
        source.export_image(scratch.0.to_str().unwrap());
        assert!(scratch.0.is_file(), "raylib exported the fixture PNG");

        let decoded = image_rgba8(&scratch.0).expect("the exported PNG decodes");
        assert_eq!((decoded.width, decoded.height), (4, 3));
        assert_eq!(decoded.pixels.len(), 4 * 3 * 4);
        assert_eq!(&decoded.pixels[..4], &[10, 20, 30, 255]);
        // Tightly packed, not merely the right length: the last pixel has to be
        // the last four bytes, which is what `ascii_art::convert_rgba8` assumes
        // when it strides by `width * 4`.
        assert_eq!(
            &decoded.pixels[decoded.pixels.len() - 4..],
            &[10, 20, 30, 255]
        );
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused_rather_than_decoded() {
        let scratch = Scratch::new("not-an-image.png");
        std::fs::write(&scratch.0, b"this is not a PNG").unwrap();
        assert_eq!(image_rgba8(&scratch.0), Err(ImageError::Decode));
    }

    #[test]
    fn a_missing_file_is_refused() {
        let mut path = std::env::temp_dir();
        path.push("musializer-decode-absent.png");
        let _ = std::fs::remove_file(&path);
        assert_eq!(image_rgba8(&path), Err(ImageError::Decode));
    }

    fn png_bytes(width: u32, height: u32, depth: u8, colour: u8) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[depth, colour, 0, 0, 0]);
        bytes
    }

    fn jpeg_bytes(width: u16, height: u16) -> Vec<u8> {
        // An APP1 first, holding what looks like another frame header, the way
        // an EXIF thumbnail does.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1, 0, 11];
        bytes.extend_from_slice(&[0xff, 0xc0, 0, 8, 8, 0, 1, 0, 1]);
        bytes.extend_from_slice(&[0xff, 0xc0, 0, 17, 8]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes
    }

    fn webp_chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend_from_slice(tag);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn headers_are_read_with_no_decoder() {
        assert_eq!(
            picture_header(&png_bytes(16384, 8192, 8, 6)),
            Some(PictureHeader {
                format: PictureFormat::Png { row_bits: 32 },
                width: 16384,
                height: 8192
            })
        );
        let jpeg = picture_header(&jpeg_bytes(4000, 3000)).unwrap();
        assert_eq!(
            (jpeg.width, jpeg.height),
            (4000, 3000),
            "not the thumbnail's"
        );
        let vp8x = webp_chunk(b"VP8X", &[0, 0, 0, 0, 0xff, 0x3f, 0, 0xff, 0x1f, 0]);
        let vp8x = picture_header(&vp8x).unwrap();
        assert_eq!((vp8x.width, vp8x.height), (16384, 8192));
        let vp8 = webp_chunk(b"VP8 ", &[0x10, 3, 0, 0x9d, 1, 0x2a, 0x40, 0, 0x20, 0]);
        let vp8 = picture_header(&vp8).unwrap();
        assert_eq!((vp8.width, vp8.height), (64, 32));
        // The bytes of a real 64 x 32 lossless file.
        let vp8l = webp_chunk(b"VP8L", &[0x2f, 0x3f, 0xc0, 0x07, 0x00]);
        let vp8l = picture_header(&vp8l).unwrap();
        assert_eq!((vp8l.width, vp8l.height), (64, 32));
    }

    #[test]
    fn a_header_that_cannot_be_read_goes_to_no_decoder() {
        for bytes in [
            &b""[..],
            b"this is not a picture",
            b"GIF89a\x10\0\x10\0",
            b"BM\0\0\0\0",
            b"\x89PNG\r\n\x1a\n",
            b"\x89PNG\r\n\x1a\n\0\0\0\x0dIDAT",
            &[0xff, 0xd8],
            &[0xff, 0xd8, 0xff, 0xda, 0, 2],
            b"RIFF\0\0\0\0WEBPVP8 ",
            b"RIFF\0\0\0\0WAVEfmt ",
        ] {
            assert_eq!(picture_header(bytes), None, "{bytes:?}");
        }
        assert_eq!(picture_header(&png_bytes(0, 10, 8, 2)), None);
        assert_eq!(
            picture_header(&png_bytes(10, 10, 8, 5)),
            None,
            "no such colour type"
        );
    }

    #[test]
    fn the_budget_is_checked_and_cannot_wrap() {
        let png = |w, h, depth, colour| picture_header(&png_bytes(w, h, depth, colour)).unwrap();
        // The bomb in the review: nothing past its IHDR is needed.
        assert!(!png(16384, 16384, 8, 6).within_budget());
        assert!(!png(u32::MAX, u32::MAX, 16, 6).within_budget());
        assert_eq!(png(u32::MAX, u32::MAX, 16, 6).working_bytes(), None);
        assert!(png(1920, 1080, 8, 6).within_budget());
        assert!(png(7680, 4320, 8, 6).within_budget());
        // Within the pixels and past the memory: 100 MP at 16 bits a channel.
        let deep = png(10_000, 10_000, 16, 6);
        assert_eq!(deep.working_bytes(), Some(800_000_000 + 800_010_000));
        assert!(!deep.within_budget());
        assert!(picture_header(&png_bytes(10_000, 10_000, 8, 0))
            .unwrap()
            .within_budget());
        assert!(!picture_header(&jpeg_bytes(65_535, 65_535))
            .unwrap()
            .within_budget());
    }

    #[test]
    fn an_oversized_header_is_refused_before_a_decoder_sees_it() {
        let scratch = Scratch::new("bomb.png");
        std::fs::write(&scratch.0, png_bytes(16384, 16384, 8, 6)).unwrap();
        let flag = AtomicBool::new(false);
        assert_eq!(
            with_picture(&scratch.0, &flag, |_, size| size),
            Err(PictureError::TooLarge)
        );
    }

    /// A blocking `open` of a FIFO nobody writes never returns, so the check
    /// runs on a thread of its own and a hang fails rather than holds the suite.
    #[test]
    fn a_fifo_under_the_name_is_refused_at_once() {
        let scratch = Scratch::new("fifo.png");
        let made = Command::new("mkfifo").arg(&scratch.0).status().unwrap();
        assert!(made.success());
        let path = scratch.0.clone();
        let (done, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let flag = AtomicBool::new(false);
            let _ = done.send(with_picture(&path, &flag, |_, size| size));
        });
        assert_eq!(
            answer.recv_timeout(Duration::from_secs(5)),
            Ok(Err(PictureError::Undecodable))
        );
    }

    #[test]
    fn a_missing_file_says_missing() {
        let flag = AtomicBool::new(false);
        let path = std::env::temp_dir().join("musializer-decode-absent-too.png");
        assert_eq!(
            with_picture(&path, &flag, |_, size| size),
            Err(PictureError::Missing)
        );
    }

    #[test]
    fn a_file_past_the_cap_is_not_read() {
        let scratch = Scratch::new("long.png");
        let file = std::fs::File::create(&scratch.0).unwrap();
        file.set_len(PICTURE_LARGEST_FILE + 1).unwrap();
        let flag = AtomicBool::new(false);
        assert_eq!(
            with_picture(&scratch.0, &flag, |_, size| size),
            Err(PictureError::TooLarge)
        );
    }

    #[test]
    fn a_raised_flag_stops_a_read() {
        let scratch = Scratch::new("cancelled.png");
        std::fs::write(&scratch.0, png_bytes(4, 4, 8, 6)).unwrap();
        let flag = AtomicBool::new(true);
        assert_eq!(
            with_picture(&scratch.0, &flag, |_, size| size),
            Err(PictureError::Cancelled)
        );
    }

    /// A stand-in FFmpeg: a shell script, which starts a grandchild the group
    /// kill has to reach, and records both.
    struct FakeFfmpeg {
        script: Scratch,
        pids: Scratch,
    }

    impl FakeFfmpeg {
        fn new(name: &str, body: &str) -> Self {
            let script = Scratch::new(&format!("{name}.sh"));
            let pids = Scratch::new(&format!("{name}.pids"));
            let text = format!("#!/bin/sh\n{body}\n");
            std::fs::write(
                &script.0,
                text.replace("PIDS", &pids.0.display().to_string()),
            )
            .unwrap();
            let chmod = Command::new("chmod")
                .arg("+x")
                .arg(&script.0)
                .status()
                .unwrap();
            assert!(chmod.success());
            Self { script, pids }
        }

        fn run(&self, deadline: Duration, flag: &AtomicBool) -> Result<Vec<u8>, PictureError> {
            ffmpeg_rgba(self.script.0.as_os_str(), &[1; 16], [2, 2], deadline, flag)
        }

        fn still_alive(&self) -> Vec<String> {
            std::fs::read_to_string(&self.pids.0)
                .unwrap_or_default()
                .split_whitespace()
                .filter(|pid| {
                    std::fs::read_to_string(format!("/proc/{pid}/stat"))
                        .is_ok_and(|stat| !stat.contains(") Z "))
                })
                .map(str::to_owned)
                .collect()
        }
    }

    const HANGS: &str = "sleep 60 &\necho $$ $! > PIDS\nwait";

    #[test]
    fn a_hung_ffmpeg_is_killed_at_its_deadline() {
        let fake = FakeFfmpeg::new("hung", HANGS);
        let flag = AtomicBool::new(false);
        let started = Instant::now();
        assert_eq!(
            fake.run(Duration::from_millis(300), &flag),
            Err(PictureError::Undecodable)
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            fake.still_alive(),
            Vec::<String>::new(),
            "the whole group went"
        );
    }

    #[test]
    fn cancelling_kills_a_running_ffmpeg() {
        let fake = FakeFfmpeg::new("cancelled", HANGS);
        let flag = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                flag.store(true, Ordering::Relaxed);
            });
            assert_eq!(
                fake.run(Duration::from_secs(60), &flag),
                Err(PictureError::Cancelled)
            );
        });
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fake.still_alive(), Vec::<String>::new());
    }

    #[test]
    fn ffmpeg_output_is_cut_at_what_the_header_promised() {
        let fake = FakeFfmpeg::new("flood", "echo $$ > PIDS\nexec yes");
        let flag = AtomicBool::new(false);
        let started = Instant::now();
        assert_eq!(
            fake.run(Duration::from_secs(30), &flag),
            Err(PictureError::Undecodable)
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "stopped by the cap, not the deadline: {:?}",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fake.still_alive(), Vec::<String>::new());
    }

    #[test]
    fn ffmpeg_is_fed_by_stdin_and_its_exact_output_is_kept() {
        let fake = FakeFfmpeg::new("echo", "cat");
        let flag = AtomicBool::new(false);
        assert_eq!(fake.run(Duration::from_secs(5), &flag), Ok(vec![1; 16]));
    }

    #[test]
    fn a_short_answer_from_ffmpeg_is_refused() {
        let fake = FakeFfmpeg::new("short", "head -c 4");
        let flag = AtomicBool::new(false);
        assert_eq!(
            fake.run(Duration::from_secs(5), &flag),
            Err(PictureError::Undecodable)
        );
    }
}
