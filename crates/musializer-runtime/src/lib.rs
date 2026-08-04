//! Unsafe and platform-sensitive work behind small, named safe APIs.
//!
//! The goal is not zero `unsafe`; it is a small number of reviewable unsafe
//! islands, each with its invariants written down next to it. Every `unsafe`
//! block in this crate carries a `SAFETY:` comment stating why it holds.
//!
//! Current inventory:
//!
//! | Island | Why unsafe | Invariant |
//! | --- | --- | --- |
//! | [`audio_bridge`] | raylib's `AudioCallback` is a bare `extern "C"` fn with no user-data pointer, so the ring must be reachable from a `static` | The callback only ever touches a lock-free SPSC ring; the ring outlives the stream because it is `'static` |
//!
//! Re-exports `raylib` so downstream crates depend on one place for the binding
//! version rather than each pinning it.

pub mod audio_bridge;
pub mod decode;
pub mod draw;
pub mod font;
pub mod halo;
pub mod preset_files;
pub mod process;
pub mod project_files;
pub mod support;

pub use raylib;
pub use raylib_sys;

/// The raylib version actually linked, from the vendored source.
pub const RAYLIB_VERSION: &str = raylib_5_5_link::RAYLIB_VERSION;

/// Keeps `raylib-5-5-link` in the link graph. Call once from `main`.
pub fn ensure_raylib_linked() -> &'static str {
    raylib_5_5_link::link_marker()
}
