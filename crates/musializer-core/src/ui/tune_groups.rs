//! Look's parameter groups (WX-5d §3.3): TYPE, MOTION, COLOUR, LIGHT, SHAPE,
//! SCENE.
//!
//! **Owner: the Look & Assist builder.** A table from each descriptor key to a
//! group, beside the descriptors rather than inside them, so
//! `scene/settings.rs` and `differential_settings.sh` stay untouched. An
//! exhaustive test over `scene::settings::descriptors()` for every scene keeps
//! a new parameter from landing ungrouped.
//!
//! Created empty by the WX-5d seam commit so no builder edits `ui/mod.rs`.
