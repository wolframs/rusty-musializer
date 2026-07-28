//! Caret, selection and editing policy for a single-line text field.
//!
//! **Owner: Agent I.** Scaffolded by the integration owner because `ui/mod.rs`
//! is a shared file, and because the split is the one this crate exists for:
//! every *decision* — where the caret lands, what a click selects, what a
//! keystroke does to the buffer — belongs here, raylib-free and headlessly
//! tested, while `musializer_app::ui::text_input` draws it and reads raylib's
//! keyboard.
//!
//! # There is no oracle for this
//!
//! The frozen C has no text entry: lyric text arrives through the analysis
//! bridge or a `.musi`, never through a caret. This is therefore **invention**,
//! and AGENTS.md's rule applies — decide, build it, and record the divergence
//! and its reason. What is *not* negotiable is what the text is allowed to be:
//! [`musializer_core::project::lyrics::validate_text`] and `TEXT_MAX_BYTES`
//! already define that, they are checked against the C, and a field that lets a
//! user type something the model will later reject is a bug in the field.
//!
//! Byte offsets, not character counts: the buffer is UTF-8 and a caret must
//! never land inside a code point. `str::is_char_boundary` is the guard.

// Agent I fills this. The scaffold exists so the module is registered before
// the fan-out branches; an empty module is not a design.
