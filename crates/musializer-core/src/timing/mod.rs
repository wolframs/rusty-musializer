//! Deterministic time and frame arithmetic.
//!
//! **Owner: Agent A.** No raylib, no encoder, no I/O: `render_export` here is
//! the transport *math* — frame counts over decoded audio frames — while the
//! FFmpeg process itself is Agent E's in `musializer-runtime`.

pub mod render_export;
pub mod track_identity;
pub mod track_timeline;
