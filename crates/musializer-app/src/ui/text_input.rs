//! The single-line text field: pixels, keyboard, and the caret's blink.
//!
//! **Owner: Agent I.** The policy half — caret movement, selection, what a
//! keystroke does to the buffer — is [`musializer_core::ui::text_edit`], so this
//! file holds only drawing and raylib input. That split is what makes the
//! editing rules assertable without a window.
//!
//! Scaffolded by the integration owner because `ui/mod.rs` is shared. This lives
//! beside `widgets` rather than inside it: a text field is the first widget here
//! with focus, a caret and a repeat timer, and folding that state into
//! [`super::widgets::Widgets`] — which every other widget shares — would give
//! every button a field's worth of state it does not want.
