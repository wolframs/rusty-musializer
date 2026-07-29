//! Child processes and filesystem edges.
//!
//! **Owner: Agent E.** There are **three** supervised child families, not two:
//! FFmpeg export, Assist analysis, and font import. Every spawned child is
//! explicitly finalized, killed when necessary, and waited/reaped. `Drop` makes
//! a best effort against abandonment; normal control flow reports cleanup
//! failures rather than hiding them.

pub mod assist;
pub mod dialogs;
pub mod ffmpeg;
pub mod font_import;
pub mod process_group;
pub mod publish;
// Agent H. One line, added rather than requested, because a module cannot
// register itself; nothing else in Band 1 adds a module here.
pub mod render_job;
