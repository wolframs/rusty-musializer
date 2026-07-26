//! The workspace shell: drawing, input, and the palette.
//!
//! **Owner: Agent F.** Distributed from `../musializer/src/plug.c` (8,682 lines,
//! the composition root), `musializer.c`, `ui_widgets.c/.h`, `ui_palette.h` and
//! `ui_theme.h`.
//!
//! The split with [`musializer_core::ui`] is the whole point and it is not
//! cosmetic. Everything that is a *decision* — a budget, a threshold, a hit test,
//! a mode ladder, a dirty guard — lives there, raylib-free and headlessly tested.
//! Only pixels live here. That is the practice that grew the C suite from 175 to
//! 327 tests, and the trap it avoids is the named one: what worked in C was
//! moving state out of the shell, not moving drawing code between files.
//!
//! [`shell_layout`] is the exception worth pointing at: it is raylib-free and
//! tested, but it lives app-side because `musializer_core::ui`'s module list is
//! fixed by the ownership map and this geometry belongs to nobody else.

pub mod shell;
pub mod shell_layout;
pub mod theme;
pub mod widgets;
