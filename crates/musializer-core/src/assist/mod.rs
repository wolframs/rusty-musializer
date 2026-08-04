//! Assistance provider configuration: contracts, settings, credentials.
//!
//! Design authority: `docs/ASSIST_PROVIDER_CONTRACTS.md` (P0). Everything here
//! is pure — the schema, the boundary ladder, the validation rules, and the
//! in-memory secret type. The filesystem half (path resolution, `0600`/`0700`
//! modes, atomic replacement, the startup environment import) is
//! `musializer_runtime::assist`, for the same reason `project::io` has no
//! `atomic_write`: this crate opens no files.
//!
//! There is no C oracle for any of this. The frozen binary had one hard-coded
//! OpenRouter model and read its key from the repository `.env`; the contract
//! ladder, the routing table and the credentials file are new work accepted by
//! the operator on 2026-08-04.

pub mod contracts;
pub mod credentials;
pub mod models_dir;
pub mod secret;
pub mod settings;
pub mod suitability;
