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

use std::path::Path;

use raylib::audio::{RaylibAudio, Wave};
use raylib::consts::PixelFormat;
use raylib::texture::Image;

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
/// (`raylib-5.5.1/src/core/audio.rs:261-268`), but `LoadWaveSamples` allocates
/// and fills `frameCount * channels` floats
/// (`vendor/raylib-5.5/src/raudio.c:1299-1310`). For any stereo track the safe
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
}
