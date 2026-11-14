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
    let fetched = client::Endpoint::from_env().and_then(|endpoint| {
        let destination = store
            .work("prices")
            .map(|folder| folder.join("models.json"))
            .unwrap_or_else(|_| std::env::temp_dir().join("musializer-video-models.json"));
        let stop = std::sync::atomic::AtomicBool::new(false);
        let bytes = client::fetch_models(&endpoint, &destination, &stop)
            .map_err(|error| error.to_string())?;
        store.clear_work("prices");
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
        Err(reason) => {
            let line = format!("the price list could not be read: {reason}");
            (cached, line)
        }
    }
}
