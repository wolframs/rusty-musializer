//! The drawing halves of the scenes.
//!
//! Deterministic scene state lives in `musializer-core::scene`; only the code
//! that needs raylib is here. Agents C and D own five scenes each; this module
//! is where their drawing lands.

pub mod spectrum;
