//! One file per bottom panel and per inspector pane.
//!
//! **This split exists for the fan-out, not for tidiness.** Six agents fill six
//! surfaces at once, and in session 1 the thing that prevented collisions was
//! pre-creating every file with its `pub mod` line already registered, so no
//! agent ever edits a `mod.rs`. The same rule applies here and to
//! `super::shell`, `super::widgets` and `super::theme`: an agent that needs a
//! new widget or colour **requests** it rather than adding it, because those are
//! the files every agent touches.
//!
//! Each panel is an `impl Shell` block in its own module. Rust allows inherent
//! impls to be split across modules of the same crate, so a panel gets a normal
//! method with access to shell state without anything being made public.
//!
//! # The seams
//!
//! Three surfaces nest inside another agent's, and the call sites are defined
//! here rather than negotiated later:
//!
//! | Surface | Owner | Called from |
//! | --- | --- | --- |
//! | the route editor row | G | [`tune`]'s per-setting row loop |
//! | the font browser pane | K | [`lyrics`]'s three-pane editor |
//! | the manual event row | L | [`events`], from the timeline strip |

pub mod assist;
pub mod events;
pub mod export;
pub mod fonts;
pub mod lyrics;
pub mod tune;

// `stub()` used to live here: one shared "not built yet" box, so every unfilled
// surface said so the same way. Every panel it served — Export, Lyrics, Assist,
// and the font browser pane — is now real, so it has no callers and is gone.
// That deletion was the point of sharing it: the last call disappearing is a
// visible, checkable event rather than a slow fade.
