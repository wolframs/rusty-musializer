//! Pure UI layout and editor state.
//!
//! **Owner: Agent F.** These modules are raylib-free and headlessly tested in C,
//! and they stay that way here — that is what makes layout assertable without a
//! window. Drawing lives in `musializer-app`.
//!
//! Two layout rules the C repository already paid for: a panel that reserves
//! height it never draws steals it from the scene preview, so rows only some
//! modes have are parameters rather than assumptions; and a panel's minimum size
//! is measured against what the minimum supported window (960x640) actually
//! produces, not against a guessed threshold.

pub mod assist_ui_state;
pub mod contrast;
pub mod font_import_state;
pub mod lyric_lane_edit;
pub mod lyrics_editor_layout;
pub mod notice;
pub mod route_editor_state;
pub mod row_typography;
pub mod scroll_list;
pub mod timeline_layout;
pub mod timeline_view;
pub mod workspace_layout;
