//! The drawing halves of the scenes.
//!
//! **Owned per file:** Agent C owns the first five, Agent D the last five.
//! This `mod.rs` belongs to the integration owner.
//!
//! Deterministic state lives in `musializer_core::scenes`; only code that needs
//! raylib belongs here. Both scene agents draw lines through
//! `musializer_runtime::draw` rather than each inventing line drawing.

pub mod ascii_field;
pub mod cadence;
pub mod constellation;
pub mod loom;
pub mod orbital_lattice;
pub mod pentagram;
pub mod pulse_field;
pub mod song_atlas;
pub mod spectral_terrarium;
pub mod spectrum;
