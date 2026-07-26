//! A bounded single-producer/single-consumer queue for realtime audio frames.
//!
//! Port of `../musializer/src/sample_ring.c` and `.h`. The contract that matters
//! is the one the C header states: capacity must be a power of two (which also
//! makes index rollover safe), overflow drops the *new* frame rather than
//! overwriting queued audio, and push/pop perform no allocation, locking, I/O,
//! or blocking.
//!
//! The Rust shape differs from C in one deliberate way: C takes caller-owned
//! storage (`SampleRing.storage` is a raw pointer) because it must live in
//! hot-reloadable state without an allocator. Hot reload is a stated non-goal
//! here, so this owns a boxed slice instead and the unsafe pointer juggling goes
//! away. The realtime producer still allocates nothing — allocation happens once
//! at construction.

use std::sync::atomic::{AtomicUsize, Ordering};

/// One stereo frame. Matches C's `SampleFrame` (`sample_ring.h:9-12`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct SampleFrame {
    pub left: f32,
    pub right: f32,
}

impl SampleFrame {
    pub const SILENT: Self = Self {
        left: 0.0,
        right: 0.0,
    };

    #[must_use]
    pub fn new(left: f32, right: f32) -> Self {
        Self { left, right }
    }

    /// The mono mix raylib's stereo mixing format reduces to for analysis.
    #[must_use]
    pub fn mono(self) -> f32 {
        (self.left + self.right) * 0.5
    }
}

/// Bounded SPSC queue over a power-of-two backing store.
///
/// One producer (the audio callback thread) calls [`SampleRing::push`] /
/// [`SampleRing::push_many`]; one consumer (the frame loop) calls
/// [`SampleRing::pop`] / [`SampleRing::pop_many`]. Any other pairing is a bug
/// this type does not defend against, exactly as in C.
#[derive(Debug)]
pub struct SampleRing {
    storage: Box<[SyncCell]>,
    capacity: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicUsize,
}

/// A cell the producer writes and the consumer reads.
///
/// `UnsafeCell` is what makes the SPSC discipline expressible: the slot's
/// synchronisation comes from the acquire/release pair on `head`/`tail`, not
/// from the cell itself, so the data needs interior mutability without a lock.
#[derive(Debug)]
struct SyncCell(std::cell::UnsafeCell<SampleFrame>);

// SAFETY: A cell is only ever written by the producer while the consumer is
// forbidden from reading it (the consumer stops at `head`, which the producer
// publishes with Release *after* the write), and only ever read by the consumer
// while the producer is forbidden from writing it (the producer stops at
// `tail + capacity`, which the consumer publishes with Release after the read).
// So no two threads ever touch one cell at the same time.
unsafe impl Sync for SyncCell {}
unsafe impl Send for SyncCell {}

/// Rejects a capacity C's `sample_ring_init` would reject (`sample_ring.c:5-24`).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SampleRingError {
    #[error("sample ring capacity must be non-zero")]
    ZeroCapacity,
    #[error("sample ring capacity must be a power of two, got {0}")]
    NotPowerOfTwo(usize),
}

impl SampleRing {
    /// # Errors
    /// Rejects a zero or non-power-of-two capacity, matching C.
    pub fn with_capacity(capacity: usize) -> Result<Self, SampleRingError> {
        if capacity == 0 {
            return Err(SampleRingError::ZeroCapacity);
        }
        if !capacity.is_power_of_two() {
            return Err(SampleRingError::NotPowerOfTwo(capacity));
        }
        let storage = (0..capacity)
            .map(|_| SyncCell(std::cell::UnsafeCell::new(SampleFrame::SILENT)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            storage,
            capacity,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        })
    }

    /// Only call while producer and consumer are stopped (`sample_ring.c:26-32`).
    pub fn reset(&self) {
        self.head.store(0, Ordering::Relaxed);
        self.tail.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
    }

    /// Pushes one frame. Returns `false` and counts a drop when full.
    ///
    /// Realtime-safe: no allocation, no lock, no syscall. Port of
    /// `sample_ring_push` (`sample_ring.c:34-46`) including its memory orders.
    pub fn push(&self, frame: SampleFrame) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.capacity {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let slot = &self.storage[head & (self.capacity - 1)];
        // SAFETY: see the `unsafe impl Sync for SyncCell` note. This index is in
        // [tail, tail + capacity), which the consumer will not read until the
        // Release store below publishes it.
        unsafe { *slot.0.get() = frame };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Pops one frame, or `None` when empty (`sample_ring.c:48-59`).
    pub fn pop(&self) -> Option<SampleFrame> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot = &self.storage[tail & (self.capacity - 1)];
        // SAFETY: this index is in [tail, head), which the producer published
        // with a Release store and will not write again until the consumer's
        // Release store below frees the slot.
        let frame = unsafe { *slot.0.get() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(frame)
    }

    /// Pushes as many frames as fit, returning how many were taken.
    ///
    /// Matches C's drop accounting exactly: `push` already counted the first
    /// rejection, so only the remainder is added (`sample_ring.c:61-71`).
    pub fn push_many(&self, frames: &[SampleFrame]) -> usize {
        let mut pushed = 0;
        while pushed < frames.len() && self.push(frames[pushed]) {
            pushed += 1;
        }
        if pushed < frames.len() {
            self.dropped
                .fetch_add(frames.len() - pushed - 1, Ordering::Relaxed);
        }
        pushed
    }

    /// Pops into `out`, returning how many frames were written.
    pub fn pop_many(&self, out: &mut [SampleFrame]) -> usize {
        let mut popped = 0;
        while popped < out.len() {
            match self.pop() {
                Some(frame) => {
                    out[popped] = frame;
                    popped += 1;
                }
                None => break,
            }
        }
        popped
    }

    /// Queued frame count, clamped to capacity as in C (`sample_ring.c:81-88`).
    #[must_use]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail).min(self.capacity)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Frames dropped because the queue was full. Never resets except in
    /// [`SampleRing::reset`].
    #[must_use]
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_capacities_c_rejects() {
        assert_eq!(
            SampleRing::with_capacity(0).unwrap_err(),
            SampleRingError::ZeroCapacity
        );
        assert_eq!(
            SampleRing::with_capacity(3).unwrap_err(),
            SampleRingError::NotPowerOfTwo(3)
        );
        assert!(SampleRing::with_capacity(1).is_ok());
        assert!(SampleRing::with_capacity(4096).is_ok());
    }

    #[test]
    fn overflow_drops_new_frames_and_never_overwrites_queued_audio() {
        let ring = SampleRing::with_capacity(2).unwrap();
        assert!(ring.push(SampleFrame::new(1.0, 1.0)));
        assert!(ring.push(SampleFrame::new(2.0, 2.0)));
        assert!(!ring.push(SampleFrame::new(3.0, 3.0)));
        assert_eq!(ring.dropped(), 1);
        // The queued audio is the first two frames, not the newest two.
        assert_eq!(ring.pop(), Some(SampleFrame::new(1.0, 1.0)));
        assert_eq!(ring.pop(), Some(SampleFrame::new(2.0, 2.0)));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn push_many_drop_accounting_matches_c() {
        let ring = SampleRing::with_capacity(2).unwrap();
        let frames = [SampleFrame::new(1.0, 0.0); 5];
        assert_eq!(ring.push_many(&frames), 2);
        // Three frames did not fit; push() counted one, push_many() the other two.
        assert_eq!(ring.dropped(), 3);
    }

    #[test]
    fn indices_survive_rollover() {
        let ring = SampleRing::with_capacity(4).unwrap();
        for round in 0..1000u32 {
            let frame = SampleFrame::new(round as f32, 0.0);
            assert!(ring.push(frame));
            assert_eq!(ring.pop(), Some(frame));
        }
        assert!(ring.is_empty());
        assert_eq!(ring.dropped(), 0);
    }

    #[test]
    fn survives_a_real_producer_and_consumer() {
        use std::sync::Arc;
        let ring = Arc::new(SampleRing::with_capacity(64).unwrap());
        let producer = {
            let ring = Arc::clone(&ring);
            std::thread::spawn(move || {
                let mut sent = 0usize;
                for i in 0..100_000u32 {
                    if ring.push(SampleFrame::new(i as f32, -(i as f32))) {
                        sent += 1;
                    }
                }
                sent
            })
        };
        let consumer = {
            let ring = Arc::clone(&ring);
            std::thread::spawn(move || {
                let mut got = 0usize;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while std::time::Instant::now() < deadline {
                    match ring.pop() {
                        // Whatever comes out must be a frame that went in, not a
                        // torn read of two.
                        Some(frame) => {
                            assert_eq!(frame.left, -frame.right);
                            got += 1;
                        }
                        None if got > 0 && ring.dropped() + got >= 100_000 => break,
                        None => std::hint::spin_loop(),
                    }
                }
                got
            })
        };
        let sent = producer.join().unwrap();
        let got = consumer.join().unwrap();
        assert_eq!(sent, got + ring.len());
        assert_eq!(sent + ring.dropped(), 100_000);
    }
}
