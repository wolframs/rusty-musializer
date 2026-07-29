//! The realtime audio callback bridge.
//!
//! raylib invokes a stream processor on its audio thread. The invariant this
//! module exists to hold is the plan's: **audio callbacks do not allocate,
//! block, perform file I/O, or touch UI state.** All the callback does is copy
//! interleaved stereo frames into a lock-free SPSC ring that the frame loop
//! drains.
//!
//! ## Why there is a `static`
//!
//! raylib 5.5's `AudioCallback` is `void (*)(void *bufferData, unsigned int
//! frames)` — there is no user-data pointer, so a closure cannot be attached.
//! The C oracle solves this the same way, with a file-scope callback reaching
//! global state (`../musializer/src/plug.c:531-536`). The ring therefore lives in
//! a `static`, which is what makes this an unsafe island rather than an ordinary
//! safe wrapper.
//!
//! The alternative — `raylib`'s own `attach_audio_stream_processor_to_music` —
//! was rejected deliberately: it routes every callback through a
//! `LazyLock<Mutex<..>>` slot table, and taking a mutex on the audio thread is
//! exactly what the invariant forbids.
//!
//! ## What the callback receives
//!
//! Interleaved `f32` **stereo** frames, always: raylib runs stream processors
//! after conversion to its mixing format, which is 2 channels in the vendored
//! configuration, regardless of the source file's channel count
//! (`../musializer/src/plug.c:533-534`).

use std::os::raw::{c_uint, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use musializer_core::audio::{SampleFrame, SampleRing};

/// Channels raylib mixes to before invoking a stream processor.
pub const MIXED_CHANNELS: usize = 2;

/// Default ring capacity, in stereo frames. Must be a power of two.
///
/// 8192 frames is ~186 ms at 44.1 kHz — comfortably more than a frame's worth of
/// audio at any sane frame rate, so the consumer never starves and the producer
/// never drops during normal playback.
pub const DEFAULT_CAPACITY: usize = 8192;

/// The single ring the callback writes into.
static CAPTURE_RING: OnceLock<SampleRing> = OnceLock::new();

/// Whether the callback should be writing. Cleared before detaching so a
/// callback already in flight becomes a no-op.
static CAPTURING: AtomicBool = AtomicBool::new(false);

/// Installing or attaching went wrong.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("the audio bridge is already installed")]
    AlreadyInstalled,
    #[error("the audio bridge is not installed")]
    NotInstalled,
    #[error("invalid ring capacity: {0}")]
    Capacity(#[from] musializer_core::audio::sample_ring::SampleRingError),
}

/// Installs the capture ring. Call once, before attaching to any stream.
///
/// Allocation happens here and only here — the callback itself never allocates.
pub fn install(capacity: usize) -> Result<(), BridgeError> {
    let ring = SampleRing::with_capacity(capacity)?;
    CAPTURE_RING
        .set(ring)
        .map_err(|_| BridgeError::AlreadyInstalled)
}

/// Installs with [`DEFAULT_CAPACITY`], succeeding if already installed.
///
/// Convenient for a `main` that does not care whether an earlier call won.
pub fn install_default() {
    let _ = install(DEFAULT_CAPACITY);
}

/// The capture ring, if installed.
#[must_use]
pub fn ring() -> Option<&'static SampleRing> {
    CAPTURE_RING.get()
}

/// The stream processor raylib calls on its audio thread.
///
/// Realtime-safe by construction: one bounds check, one `OnceLock` acquire load,
/// a slice read, and N lock-free pushes. No allocation, no lock, no syscall, no
/// panic path that can unwind into C.
///
/// # Safety
/// raylib guarantees `buffer_data` points to `frames * MIXED_CHANNELS` `f32`
/// values for the duration of the call. This is only ever installed through
/// [`attach`], which is the function that documents that contract.
unsafe extern "C" fn capture_callback(buffer_data: *mut c_void, frames: c_uint) {
    if !CAPTURING.load(Ordering::Relaxed) {
        return;
    }
    if buffer_data.is_null() || frames == 0 {
        return;
    }
    let Some(ring) = CAPTURE_RING.get() else {
        return;
    };

    let sample_count = frames as usize * MIXED_CHANNELS;
    // SAFETY: raylib passes `frames` stereo f32 frames in `buffer_data`, and the
    // pointer is valid for the duration of this call. We only read.
    let samples = unsafe { std::slice::from_raw_parts(buffer_data as *const f32, sample_count) };

    for frame in samples.chunks_exact(MIXED_CHANNELS) {
        // Push per frame rather than building a temporary buffer: no stack
        // array to size, and a full ring simply drops as the contract says.
        if !ring.push(SampleFrame::new(frame[0], frame[1])) {
            // Ring is full. `push` has already counted the drop; queued audio
            // is never overwritten, so stop rather than spin.
            break;
        }
    }
}

/// Attaches the capture callback to a raylib audio stream.
///
/// # Safety
/// - [`install`] must have been called, and the audio device must be initialized.
/// - `stream` must be a valid, live `AudioStream` — in practice `*music` for a
///   `Music` that is still alive.
/// - The caller must call [`detach`] with the same stream before the stream is
///   unloaded. raylib keeps a raw pointer to the processor list per stream; a
///   detached-too-late callback is a use-after-free in C.
pub unsafe fn attach(stream: raylib_sys::AudioStream) -> Result<(), BridgeError> {
    if CAPTURE_RING.get().is_none() {
        return Err(BridgeError::NotInstalled);
    }
    CAPTURING.store(true, Ordering::Release);
    // SAFETY: the caller's contract above covers stream validity; the callback
    // is a plain `extern "C"` fn with no captured state.
    unsafe {
        raylib_sys::AttachAudioStreamProcessor(stream, Some(capture_callback));
    }
    Ok(())
}

/// Detaches the capture callback.
///
/// Stops capturing *before* detaching, so a callback already running on the
/// audio thread becomes a no-op rather than racing the detach.
///
/// # Safety
/// `stream` must be the same still-valid stream passed to [`attach`].
pub unsafe fn detach(stream: raylib_sys::AudioStream) {
    CAPTURING.store(false, Ordering::Release);
    // SAFETY: caller guarantees the stream is the one attached and still valid.
    unsafe {
        raylib_sys::DetachAudioStreamProcessor(stream, Some(capture_callback));
    }
}

/// Whether the callback is currently writing frames.
#[must_use]
pub fn is_capturing() -> bool {
    CAPTURING.load(Ordering::Relaxed)
}

/// Drains queued frames into an analyzer-ready interleaved buffer.
///
/// Returns the number of *frames* written. `out` must be at least
/// `2 * frame_capacity` long; the analyzer consumes interleaved stereo.
pub fn drain_interleaved(out: &mut [f32]) -> usize {
    let Some(ring) = CAPTURE_RING.get() else {
        return 0;
    };
    let mut frames = 0;
    let capacity = out.len() / MIXED_CHANNELS;
    while frames < capacity {
        match ring.pop() {
            Some(frame) => {
                out[frames * MIXED_CHANNELS] = frame.left;
                out[frames * MIXED_CHANNELS + 1] = frame.right;
                frames += 1;
            }
            None => break,
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// Every test here drives the **process-global** ring and `CAPTURING` flag,
    /// so they cannot run concurrently with each other — one test's `reset` is
    /// another's missing frames. `cargo test` runs a binary's tests in parallel
    /// by default, and this suite got large enough during the Band 1 fan-out for
    /// that to start failing intermittently rather than never.
    ///
    /// A lock rather than `--test-threads=1`, because the fix has to hold for
    /// anyone who runs `cargo test` the ordinary way.
    static BRIDGE: Mutex<()> = Mutex::new(());

    fn exclusive() -> MutexGuard<'static, ()> {
        // A poisoned lock means a previous test panicked while holding it; the
        // state it guards is rebuilt by `install_default` in every test anyway.
        BRIDGE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The callback is exercised directly here rather than through raylib: no
    /// audio device, no window, no sound card. That is the point of the design —
    /// the realtime path is testable without a session.
    #[test]
    fn the_callback_moves_stereo_frames_into_the_ring() {
        let _guard = exclusive();
        install_default();
        let ring = ring().expect("installed");
        ring.reset();
        CAPTURING.store(true, Ordering::Release);

        let mut buffer = [0.25f32, -0.25, 0.5, -0.5, 0.75, -0.75];
        // SAFETY: buffer holds exactly 3 stereo frames, matching the count.
        unsafe { capture_callback(buffer.as_mut_ptr() as *mut c_void, 3) };

        assert_eq!(ring.len(), 3);
        assert_eq!(ring.pop(), Some(SampleFrame::new(0.25, -0.25)));
        assert_eq!(ring.pop(), Some(SampleFrame::new(0.5, -0.5)));
        assert_eq!(ring.pop(), Some(SampleFrame::new(0.75, -0.75)));

        CAPTURING.store(false, Ordering::Release);
    }

    #[test]
    fn a_null_buffer_or_zero_frames_is_ignored() {
        let _guard = exclusive();
        install_default();
        let ring = ring().expect("installed");
        ring.reset();
        CAPTURING.store(true, Ordering::Release);

        // SAFETY: both calls take the early-return paths and dereference nothing.
        unsafe {
            capture_callback(std::ptr::null_mut(), 4);
            let mut buffer = [0.0f32; 2];
            capture_callback(buffer.as_mut_ptr() as *mut c_void, 0);
        }
        assert!(ring.is_empty());

        CAPTURING.store(false, Ordering::Release);
    }

    #[test]
    fn a_detached_bridge_drops_frames_instead_of_writing_them() {
        let _guard = exclusive();
        install_default();
        let ring = ring().expect("installed");
        ring.reset();
        CAPTURING.store(false, Ordering::Release);

        let mut buffer = [1.0f32, 1.0];
        // SAFETY: one valid stereo frame; the callback returns early anyway.
        unsafe { capture_callback(buffer.as_mut_ptr() as *mut c_void, 1) };
        assert!(ring.is_empty(), "a detached bridge must not touch the ring");
    }

    #[test]
    fn draining_produces_interleaved_samples() {
        let _guard = exclusive();
        install_default();
        let ring = ring().expect("installed");
        ring.reset();
        ring.push(SampleFrame::new(0.1, 0.2));
        ring.push(SampleFrame::new(0.3, 0.4));

        let mut out = [0.0f32; 8];
        assert_eq!(drain_interleaved(&mut out), 2);
        assert_eq!(&out[..4], &[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(drain_interleaved(&mut out), 0, "the ring is empty now");
    }

    #[test]
    fn the_default_capacity_is_a_valid_ring_size() {
        assert!(DEFAULT_CAPACITY.is_power_of_two());
        assert!(SampleRing::with_capacity(DEFAULT_CAPACITY).is_ok());
    }
}
