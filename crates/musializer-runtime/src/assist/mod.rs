//! The filesystem and environment edges of assist provider configuration.
//!
//! The schema, the boundary ladder and the in-memory secret type are
//! [`musializer_core::assist`], which opens nothing. What is here is what that
//! crate is not allowed to do: resolve XDG paths, set `0600` and `0700` modes
//! before secret bytes exist, replace a file atomically, and take the one
//! authorized credential out of this process's environment at startup.
//!
//! Design authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` §2, §3 and §4 (E1).

pub mod env;
pub mod files;
pub mod models;
