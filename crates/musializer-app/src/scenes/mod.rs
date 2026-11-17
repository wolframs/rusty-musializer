//! The drawing halves of the scenes.
//!
//! Deterministic state lives in `musializer_core::scenes`; only code that needs
//! raylib belongs here. Scene modules share `musializer_runtime::draw`, while
//! the application-side caption composition lives beside them.

pub mod ascii_field;
pub mod cadence;
pub mod caption;
/// Post-legacy (2026-08-24), no oracle. See its core module for its evidence.
pub mod clawd;
/// Post-legacy (2026-11-14, VG): a video file as a section's picture.
pub mod clip;
pub mod constellation;
pub mod loom;
pub mod orbital_lattice;
pub mod pentagram;
pub mod phosphor_dream;
pub mod pulse_field;
pub mod song_atlas;
pub mod spectral_terrarium;
pub mod spectrum;

pub mod aurora_veil;
pub mod fluid_ink;
/// Post-legacy (2026-10-24): a wireframe solid lit on a honeycomb of glyphs.
pub mod glyph_lattice;
pub mod iridescent_pool;
pub mod liquid_chrome;
pub mod lumen_knot;
pub mod metaball_garden;
pub mod neon_tunnel;
/// Post-legacy (2026-10-24): the six pixel-art stagings' shared drawing half.
pub mod pixel_cats;
pub mod stage;
pub mod threshold;
pub mod voronoi_bloom;
