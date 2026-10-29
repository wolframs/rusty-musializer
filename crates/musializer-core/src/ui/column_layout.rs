//! The right column's geometry (WX-5d, `build/redesign/wx5d-design.md` §1).
//!
//! **Owner: the Frame builder.** Raylib-free, so the width rule — one width for
//! every view, `clamp(round(0.28 × window), 320, 460)` and never less than a
//! 640 px workspace, a user drag clamped to 300–560 — is tested here and
//! pinned post-legacy by `layout_dump`, not photographed.
//!
//! Created empty by the WX-5d seam commit so no builder edits `ui/mod.rs`.
