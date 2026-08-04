//! A provider credential in memory: one owner, no copies, best-effort erasure.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §3 ("in memory") and §4 (E8, E9). The
//! defensible claim is *minimized scope and no durable plaintext copy* — not
//! that a managed runtime erased every copy. That is why every guarantee here is
//! named as what it prevents:
//!
//! - **no `Clone`.** A second owner is a second lifetime to reason about, and
//!   the type is deliberately awkward to duplicate. Pinned by a test.
//! - **no derived `Debug`, no `Display`.** The hand-written `Debug` prints
//!   `<redacted>`, so a `{:?}` in a panic payload, a log line, a
//!   `#[derive(Debug)]` on a struct that holds one, or a `dbg!` left in by
//!   accident cannot spell the key (E8).
//! - **zeroized on drop.** Best effort, and the qualifier is honest: the
//!   allocator may already have copied the bytes, and a `String` grown into this
//!   type left its old buffer behind. What this does erase is the live copy.
//!
//! ## Why the erasure is a plain loop and not `write_volatile`
//!
//! `musializer-core` has no `unsafe` block anywhere, and this is not worth being
//! the first one. `std::hint::black_box` makes the zeroed buffer observable to
//! the optimizer, which is what stops the stores being eliminated as dead — the
//! same outcome a volatile write buys, without an unsafe island in the pure
//! crate. If a later reader wants literal volatile semantics, the change is one
//! loop body plus an `AGENTS.md` inventory row; it is a deliberate choice, not
//! an oversight.

use std::fmt;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use crate::project::sha256;

/// How many hex characters of `sha256(secret)` the interface may display.
///
/// §3: "provider `label` plus `sha256(secret)[0..8]`. Never any character of the
/// key, never a length, never a masked-but-selectable field."
pub const FINGERPRINT_LENGTH: usize = 8;

/// The longest credential this application will hold.
///
/// A bound rather than a guess: it exists so a hostile or corrupt credentials
/// file cannot make the process allocate, and so the value is refused at the
/// edge rather than truncated somewhere later.
pub const MAX_SECRET_BYTES: usize = 4096;

/// A credential. See the module comment for what the type does and does not
/// promise.
pub struct Secret(Vec<u8>);

impl Secret {
    /// Takes ownership of the credential.
    ///
    /// `String::into_bytes` reuses the same allocation, so the caller's buffer
    /// *is* this buffer and gets zeroized with it. Passing a `&str` instead
    /// would leave the original copy behind untouched, which is why there is no
    /// `From<&str>`.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value.into_bytes())
    }

    /// Reads the credential. Named to be conspicuous at every call site: this is
    /// the one place a key becomes ordinary data again, so a reviewer can grep
    /// for it.
    #[must_use]
    pub fn expose(&self) -> &str {
        // Every constructor takes a `String` or validates UTF-8, and nothing
        // mutates the buffer afterwards. The message carries no bytes of the
        // secret, which is the property that matters if it ever does fire.
        std::str::from_utf8(&self.0).expect("secret holds validated UTF-8")
    }

    /// `sha256(secret)[0..8]`, lowercase hex. The only identity of a key that
    /// may be displayed, logged, or written to `assist.json` (§3, E11).
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut hex = sha256::digest_hex(&self.0);
        hex.truncate(FINGERPRINT_LENGTH);
        hex
    }

    /// Whether the credential is empty. Deliberately not `len()`: §3 forbids
    /// displaying a length, and an API that hands one out invites it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Constant-time equality, so a comparison cannot become an oracle for the
    /// bytes. Not `PartialEq`, because `==` on a secret should be a decision.
    #[must_use]
    pub fn constant_time_eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        let mut difference = 0u8;
        for (left, right) in self.0.iter().zip(other.0.iter()) {
            difference |= left ^ right;
        }
        std::hint::black_box(difference) == 0
    }
}

/// Prints `<redacted>` and nothing else, so `{:?}` on any struct that holds one
/// stays safe (E8).
impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// Best-effort erasure. See the module comment for the honest scope of the
/// claim and for why this is a plain loop.
impl Drop for Secret {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            *byte = 0;
        }
        // Makes the zeroed buffer observable, so the stores above are not dead
        // code the optimizer may delete.
        let _ = std::hint::black_box(&self.0);
    }
}

/// Serializes as the bare string.
///
/// The **only** authorized sink is the `0600` credentials file
/// (`musializer.assist-credentials/v1`). Nothing else in this application may
/// serialize a `Secret`: `assist.json` carries a `lookup_id` and a fingerprint
/// (E11), `.musi` carries neither (E5), and a snapshot carries
/// `credential_present` plus a fingerprint (E4).
impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() > MAX_SECRET_BYTES {
            return Err(serde::de::Error::custom("credential is implausibly long"));
        }
        Ok(Self::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANTED: &str = "sk-or-v1-MUSICANARY7Q4X2ZK9deadbeef";

    #[test]
    fn debug_output_carries_no_character_of_the_key() {
        let secret = Secret::new(PLANTED.to_string());
        let printed = format!("{secret:?}");
        assert_eq!(printed, "<redacted>");
        assert!(!printed.contains("MUSICANARY"));
        assert!(!printed.contains("sk-or"));
        // A struct that derives Debug and holds one must stay safe too — that is
        // the case a reviewer cannot see by reading this file.
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            openrouter: Secret,
        }
        let holder = Holder {
            openrouter: Secret::new(PLANTED.to_string()),
        };
        let printed = format!("{holder:?}");
        assert!(printed.contains("<redacted>"));
        for window in PLANTED.as_bytes().windows(6) {
            let fragment = std::str::from_utf8(window).unwrap();
            assert!(
                !printed.contains(fragment),
                "derived Debug leaked {fragment}"
            );
        }
    }

    /// `Secret: !Clone` is a compile-time property, so it is pinned by
    /// resolution rather than by a runtime check: the inherent const exists only
    /// for `T: Clone`, and anything else falls through to the blanket trait's
    /// `false`. The `String` line is the positive control that the mechanism
    /// works at all — without it, a broken probe would report `false` forever.
    #[test]
    fn secret_is_not_clone() {
        trait NotClone {
            const IS_CLONE: bool = false;
        }
        impl<T> NotClone for T {}
        struct Probe<T>(std::marker::PhantomData<T>);
        impl<T: Clone> Probe<T> {
            const IS_CLONE: bool = true;
        }
        // Constant by construction — that is the point. Clippy's
        // `assertions_on_constants` is aimed at `assert!(true)`; here the
        // constant *is* the property under test, and it changes to `true` the
        // moment someone derives `Clone`.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(!<Probe<Secret>>::IS_CLONE);
            assert!(<Probe<String>>::IS_CLONE);
        }
    }

    #[test]
    fn fingerprint_is_eight_lowercase_hex_and_is_not_the_key() {
        let secret = Secret::new(PLANTED.to_string());
        let fingerprint = secret.fingerprint();
        assert_eq!(fingerprint.len(), FINGERPRINT_LENGTH);
        assert!(fingerprint
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));
        assert!(!PLANTED.contains(&fingerprint));
        assert_eq!(
            fingerprint,
            sha256::digest_hex(PLANTED.as_bytes())[..FINGERPRINT_LENGTH]
        );
    }

    #[test]
    fn different_keys_get_different_fingerprints() {
        let first = Secret::new("sk-or-v1-aaaa".to_string()).fingerprint();
        let second = Secret::new("sk-or-v1-aaab".to_string()).fingerprint();
        assert_ne!(first, second);
    }

    #[test]
    fn constant_time_equality_agrees_with_the_bytes() {
        let secret = Secret::new(PLANTED.to_string());
        assert!(secret.constant_time_eq(&Secret::new(PLANTED.to_string())));
        assert!(!secret.constant_time_eq(&Secret::new("sk-or-v1-other".to_string())));
        assert!(!secret.constant_time_eq(&Secret::new(format!("{PLANTED}x"))));
    }

    #[test]
    fn the_only_serialization_is_the_bare_string() {
        let secret = Secret::new(PLANTED.to_string());
        assert_eq!(
            serde_json::to_string(&secret).unwrap(),
            format!("\"{PLANTED}\"")
        );
        let parsed: Secret = serde_json::from_str(&format!("\"{PLANTED}\"")).unwrap();
        assert_eq!(parsed.expose(), PLANTED);
    }

    #[test]
    fn an_implausibly_long_credential_is_refused_rather_than_truncated() {
        let long = "a".repeat(MAX_SECRET_BYTES + 1);
        let error = serde_json::from_str::<Secret>(&format!("\"{long}\"")).unwrap_err();
        assert!(error.to_string().contains("implausibly long"));
    }
}
