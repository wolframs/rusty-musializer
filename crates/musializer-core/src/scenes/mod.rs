//! Deterministic scene state and per-frame updates.
//!
//! **Owned per file:** Agent C owns the first five, Agent D the last five.
//! This `mod.rs` belongs to the integration owner — it is pre-declared so two
//! agents never edit it at once. The drawing halves live in
//! `musializer-app::scenes`.
//!
//! Every scene here must be deterministic for the same seed, settings, and
//! frame sequence: no wall clock, no unseeded RNG, no I/O.

pub mod ascii_field;
pub mod cadence;
pub mod constellation;
pub mod loom;
pub mod orbital_lattice;
pub mod pentagram;
/// The one scene here with no oracle behind it: a post-legacy addition rather
/// than a port. See its module docs for what that changes about its evidence.
pub mod phosphor_dream;
pub mod pulse_field;
pub mod song_atlas;
pub mod spectral_terrarium;
pub mod spectrum;
