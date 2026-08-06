//! Pixel arithmetic that has no business being near a GPU handle.
//!
//! One module so far, [`resolve`], and it is here rather than in
//! `musializer-runtime` for the reason the crate exists: it is a formula whose
//! correctness is a *number*, and a number can be pinned by a unit test with no
//! window, no context and no encoder.

pub mod resolve;
