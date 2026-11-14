//! Generating a clip with a video model (post-legacy, 2026-11-14, VG H4).
//!
//! The pure half — the contract, the price list, the estimate, the request and
//! its identity — is `musializer_core::generation`. This half opens the
//! sockets and files: the OpenRouter calls ([`client`]), where clips, jobs and
//! prices are kept ([`store`]), and the job that turns a request into a clip
//! the importer has read ([`job`]). See `docs/VIDEO_GENERATION.md`.

pub mod client;
pub mod job;
pub mod store;
pub mod treatment;
pub mod window;

use musializer_core::assist::secret::Secret;
use musializer_core::generation::catalog::Catalog;

/// The OpenRouter key this application has, through the Assist credential
/// code: the `0600` file for the profile's account, else the one imported
/// from the environment at startup. A copy the job owns and drops.
///
/// # Errors
///
/// No key, or a credentials file refused for its permissions — by name, and
/// where the key is set.
pub fn credential(session: Option<&Secret>) -> Result<Secret, String> {
    use musializer_runtime::assist::plan::{self, CredentialLookup};
    let (settings, _, _) = plan::load_settings_or_defaults();
    match plan::openrouter_credential(plan::credential_lookup(&settings)) {
        CredentialLookup::Found(secret) => Ok(secret),
        CredentialLookup::Refused { path, mode } => Err(format!(
            "the key file {path} is refused{}: make it readable by you alone (chmod 600)",
            mode.map(|mode| format!(" (mode {mode:04o})"))
                .unwrap_or_default()
        )),
        CredentialLookup::Absent => session
            .map(|secret| Secret::new(secret.expose().to_owned()))
            .ok_or_else(|| NO_KEY.to_owned()),
    }
}

/// What a person reads when no key is stored.
pub const NO_KEY: &str =
    "no OpenRouter key is stored: set one in AI settings (the gear in the top bar, \
     OpenRouter key), or start the application with OPENROUTER_API_KEY set";

/// Seconds since the epoch, now.
#[must_use]
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// The price list: the store's, refreshed from the endpoint when it is
/// missing or stale and `refresh` allows it (a keyless GET that sends
/// nothing about anyone). Returns the list, if any, and what happened.
pub fn prices(store: &store::Store, refresh: bool) -> (Option<Catalog>, String) {
    let cached = store.prices();
    let now = now_unix();
    if cached
        .as_ref()
        .is_some_and(|catalog| !catalog.is_stale(now))
        || !refresh
    {
        let line = match &cached {
            Some(catalog) => format!("price list of {}", catalog.fetched_date()),
            None => "no price list".to_owned(),
        };
        return (cached, line);
    }
    prices_now(store).map_cached(cached)
}

trait KeepCached {
    fn map_cached(self, cached: Option<Catalog>) -> (Option<Catalog>, String);
}

impl KeepCached for (Option<Catalog>, String) {
    /// A failed read keeps the cached list (stale, and said so by the price).
    fn map_cached(self, cached: Option<Catalog>) -> (Option<Catalog>, String) {
        match self {
            (None, line) => (cached, line),
            fresh => fresh,
        }
    }
}

/// Reads the price list now, whatever the cache holds, and keeps it.
pub fn prices_now(store: &store::Store) -> (Option<Catalog>, String) {
    let now = now_unix();
    let fetched = client::Endpoint::from_env().and_then(|endpoint| {
        let (name, folder) = store.private_work("prices")?;
        let destination = folder.join("models.json");
        let stop = std::sync::atomic::AtomicBool::new(false);
        let fetched = client::fetch_models(&endpoint, &destination, &stop);
        store.clear_work(&name);
        let bytes = fetched.map_err(|error| error.to_string())?;
        Catalog::from_live(&bytes, now).map_err(|error| error.to_string())
    });
    match fetched {
        Ok(catalog) => {
            let line = match store.keep_prices(&catalog) {
                Ok(()) => format!("price list read now ({} models)", catalog.models.len()),
                Err(reason) => format!("price list read now, not kept: {reason}"),
            };
            (Some(catalog), line)
        }
        Err(reason) => (None, format!("the price list could not be read: {reason}")),
    }
}

/// `--vocabulary`'s `video_models`: the five picks with what each accepts and
/// what a second costs at 720p and 1080p without sound, from the price list
/// (read now when missing or stale: a keyless GET that sends nothing).
#[must_use]
pub fn vocabulary_models() -> serde_json::Value {
    use musializer_core::generation::price::{self, Price};
    use serde_json::json;
    let store = store::Store::from_env();
    let (catalog, line) = match &store {
        Ok(store) => prices(store, true),
        Err(reason) => (None, reason.clone()),
    };
    let now = now_unix();
    let models: Vec<serde_json::Value> = musializer_core::generation::PICKS
        .iter()
        .map(|pick| {
            let offer = catalog.as_ref().and_then(|catalog| catalog.offer(pick.id));
            let rate = |output: (u32, u32)| -> serde_json::Value {
                let Some(offer) = offer else {
                    return json!("price unknown: not on the price list");
                };
                let Ok(shape) = price::shape(offer, output) else {
                    return json!(format!("does not make {}x{}", output.0, output.1));
                };
                match price::estimate(catalog.as_ref(), now, pick.id, 1, &shape, 0) {
                    Price::Known(estimate) => json!({
                        "usd_per_second": estimate.rate.usd_per_second,
                        "approximate": estimate.rate.approximate,
                        "resolution": shape.resolution,
                    }),
                    Price::Unknown(reason) => json!(reason.reason()),
                }
            };
            json!({
                "id": pick.id,
                "label": pick.label,
                "why": pick.why,
                "seconds": offer.map(|offer| offer.durations.clone()),
                "resolutions": offer.map(|offer| offer.resolutions.clone()),
                "aspect_ratios": offer.map(|offer| offer.aspect_ratios.clone()),
                "frames": offer.map(|offer| offer.frames_text()),
                "per_second_1280x720": rate((1280, 720)),
                "per_second_1920x1080": rate((1920, 1080)),
            })
        })
        .collect();
    json!({
        "price_list": line,
        "stale_after_days": musializer_core::generation::catalog::STALE_AFTER_DAYS,
        "picks": models,
        "other_models": "any id on OpenRouter's /api/v1/videos/models that takes the frames asked for",
    })
}
