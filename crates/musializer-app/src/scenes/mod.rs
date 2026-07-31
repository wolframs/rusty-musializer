//! The drawing halves of the scenes.
//!
//! Deterministic state lives in `musializer_core::scenes`; only code that needs
//! raylib belongs here. Scene modules share `musializer_runtime::draw`, while
//! the application-side caption composition lives beside them.

pub mod ascii_field;
pub mod cadence;
pub mod caption;
pub mod constellation;
pub mod loom;
pub mod orbital_lattice;
pub mod pentagram;
pub mod pulse_field;
pub mod song_atlas;
pub mod spectral_terrarium;
pub mod spectrum;
