//! Generating a clip with a video model (post-legacy, 2026-11-14, VG H4): the
//! contract, the price list, the estimate, the request and its identity.
//!
//! Design authority: `docs/VIDEO_GENERATION.md` ("Generating a clip") and the
//! `TC-VIDEO` row of `docs/ASSIST_PROVIDER_CONTRACTS.md`. Everything here is
//! pure: the client that speaks to OpenRouter, the job that polls it and the
//! import that turns its download into a clip are the application's
//! (`musializer-app/src/generation/`), because they open sockets and files.
//!
//! The rules this module is the one place of:
//!
//! - **The price is computed before anything is sent,** from the live price
//!   list (`GET /api/v1/videos/models`, public and keyless) and the seconds
//!   asked for. A list that is missing or older than [`catalog::STALE_AFTER_DAYS`]
//!   is [`price::Price::Unknown`], and that word is what the user sees.
//! - **A request is identified by everything that determines it**
//!   ([`request::GenerationRequest::digest`]): the model, the prompt, the
//!   length, the shape, no audio, and the exact bytes of each frame image. The
//!   same digest is the same clip, so a re-run or a reopen finds the clip it
//!   already has and pays nothing; a changed prompt or frame is a new request.
//! - **What leaves the machine is said in words** before the call
//!   ([`request::GenerationRequest::leaves`]): the prompt and, when asked for,
//!   still frames this application rendered. Never audio, a path, or an
//!   account detail.

pub mod catalog;
pub mod price;
pub mod request;

/// The provider every generation goes through. The only one (out of scope:
/// any other, `docs/VIDEO_GENERATION.md`).
pub const PROVIDER: &str = "openrouter";

/// The contract id, in the vocabulary of `docs/ASSIST_PROVIDER_CONTRACTS.md` §1.
/// Not one of the Assist profile's routable contracts: its route is fixed
/// (OpenRouter, a model the person picks per clip), so it has no row in
/// `assist.json` and changes nothing the Assist harnesses pin.
pub const CONTRACT: &str = "TC-VIDEO";

/// One of the five picks (`docs/VIDEO_GENERATION.md`, "The five picks"): the
/// latest model of each family that makes 1080p or better and takes a first
/// and a last frame. What each accepts and costs comes from the live list; the
/// line here is why it is on the list at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub id: &'static str,
    /// A short name for a control.
    pub label: &'static str,
    pub why: &'static str,
}

/// The five picks, cheapest first as of 2026-11-14.
#[rustfmt::skip]
pub const PICKS: [Pick; 5] = [
    Pick { id: "google/veo-3.1-lite",            label: "Veo 3.1 Lite",  why: "drafts and volume; 16:9 and 9:16 only" },
    Pick { id: "minimax/hailuo-3",               label: "Hailuo 3",      why: "the cheapest 2K, every aspect" },
    Pick { id: "alibaba/wan-3.0",                label: "Wan 3.0",       why: "the longest clip, 2 to 30 s" },
    Pick { id: "black-forest-labs/flux-3-video", label: "FLUX 3 Video",  why: "up to 4K, strong keyframes" },
    Pick { id: "bytedance/seedance-2.0",         label: "Seedance 2.0",  why: "the strongest prompt adherence; priced by token" },
];

/// The pick with this id.
#[must_use]
pub fn pick(id: &str) -> Option<&'static Pick> {
    PICKS.iter().find(|pick| pick.id == id)
}

/// The longest prompt a request carries, in bytes. A bound, not a style
/// rule: the treatment, the card and the `.musi` all hold it.
pub const MAX_PROMPT_BYTES: usize = 2000;

/// The longest clip any request asks for, in seconds: the longest any pick
/// makes (Wan 3.0, 30 s).
pub const MAX_SECONDS: u32 = 30;
