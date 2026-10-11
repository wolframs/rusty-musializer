//! Pixel arithmetic that has no business being near a GPU handle.
//!
//! Two modules, [`resolve`] and [`contact_sheet`], and they are here rather
//! than in `musializer-runtime` for the reason the crate exists: each is a
//! formula whose correctness is a *number*, and a number can be pinned by a unit
//! test with no window, no context and no encoder.

pub mod contact_sheet;
pub mod resolve;
