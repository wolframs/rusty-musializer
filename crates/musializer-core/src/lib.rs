//! Pure, deterministic, headlessly testable Musializer code.
//!
//! Nothing in this crate touches raylib, the filesystem, OS processes, or
//! global mutable state. That constraint is the whole point: the C project's
//! fourteen raylib-free modules are what made its 327-test suite possible, and
//! this crate is the same bet with the language on its side.
//!
//! Ported against the frozen C oracle at `../musializer` commit `9300af9`.
//! Where a function reproduces C behaviour, the doc comment cites the C file and
//! line so a later reader can check the port rather than trust it.

pub mod audio;
pub mod scene;
