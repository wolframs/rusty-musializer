//! Link-only crate: it contributes raylib 6.0 (built from vendored source) to
//! the final binary's link, and nothing else. See `build.rs` for the reasoning.
//!
//! Depend on this crate anywhere `raylib`/`raylib-sys` symbols are used. Because
//! `raylib-sys` runs in `nobuild` mode it emits no link directives of its own,
//! so without this crate in the graph the link fails with undefined raylib
//! symbols.

/// The raylib version this crate builds and links.
pub const RAYLIB_VERSION: &str = "6.0";

/// Forces the linker to keep this crate in the graph.
///
/// A crate whose `lib.rs` is empty can be dropped by the compiler before its
/// build script's link directives matter. Calling this once from the binary is
/// the cheap, obvious way to pin it.
pub fn link_marker() -> &'static str {
    RAYLIB_VERSION
}
