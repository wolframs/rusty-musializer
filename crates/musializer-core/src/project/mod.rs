//! The `.musi` project model, codec, and editor state.
//!
//! **Owner: Agent B.** Pure and headlessly testable: no raylib, no filesystem
//! side effects beyond the transactional-save helpers' explicit paths.

pub mod analysis_bridge;
pub mod analysis_candidate;
pub mod assets;
pub mod caption_layout;
pub mod editor_draft;
pub mod event_timeline;
pub mod io;
pub mod lyrics;
pub mod model;
pub mod preset_store;
pub mod scene_switch;
pub mod semantic_lane;
pub mod sha256;
