//! The suitability overlay: which model is fit for which contract, and on what
//! evidence.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §1 says modality eligibility is
//! *necessary and not sufficient* — a model that accepts audio and emits text
//! can still be wrong for `TC-ALIGN` — and §6 records a `suitability_revision`
//! in every execution snapshot. This module is the thing that revision names.
//!
//! Three properties are the whole design, and each has a test:
//!
//! - **An absent model is `experimental`, never `recommended`.** The table is
//!   an allowlist of *evidence*, not of models: a model nobody has measured is
//!   usable behind `catalog.show_experimental` and is never offered by default.
//! - **A `recommended` row must carry an evidence date and the prompt/schema
//!   version it was benchmarked at.** Without those, "recommended" is an
//!   opinion, and §6 says a run whose provenance is inexact "must not be
//!   accepted as benchmark evidence". The test enforces it on the table, so a
//!   future row cannot be promoted by editing one word.
//! - **Rows are seeded only from evidence this repository holds.** Every row
//!   below cites `docs/LYRICS_TIMING_BENCHMARK_RESULTS.md` or names itself
//!   unmeasured. There are three rows because there are three measurements; the
//!   table is short on purpose.
//!
//! It is a `const` table rather than a JSON asset because `musializer-core`
//! opens no files, and because a mistyped contract id should be a compile error
//! rather than a lookup that silently returns `experimental` forever.

use serde::{Deserialize, Serialize};

use crate::assist::contracts::ContractId;

/// The overlay's version, recorded as `suitability_revision` in an execution
/// snapshot (§6). Bump the date when a row changes so an old snapshot stays
/// interpretable.
pub const OVERLAY_REVISION: &str = "musializer.assist-suitability/2026-08-04";

/// How fit a model is for a contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Suitability {
    /// Measured on this repository's own evidence and offered by default.
    Recommended,
    /// Usable, but unmeasured or only partly measured. Hidden unless
    /// `catalog.show_experimental` is on.
    Experimental,
    /// Measured and failed, or known to be wrong for the contract. Never
    /// offered, however the picker is configured.
    Unsupported,
}

impl Suitability {
    /// The stable token, for files and for the snapshot.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Recommended => "recommended",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
        }
    }

    /// Parses a token. Unknown tokens are refused rather than defaulted; a
    /// defaulted status would be a silent promotion.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        [Self::Recommended, Self::Experimental, Self::Unsupported]
            .into_iter()
            .find(|candidate| candidate.token() == token)
    }

    /// Whether a picker shows this model without being asked twice.
    /// `show_experimental` is `catalog.show_experimental` from §2.
    #[must_use]
    pub const fn offered(self, show_experimental: bool) -> bool {
        match self {
            Self::Recommended => true,
            Self::Experimental => show_experimental,
            Self::Unsupported => false,
        }
    }
}

/// How much audio the route sends to the model.
///
/// The tokens are §6's `audio_scope`. For a `local-only` contract nothing
/// leaves the machine whatever this says — the field describes what the model
/// *consumes*, and the boundary ladder in `contracts.rs` is what decides where
/// it may travel. Recording it here is what lets a picker warn before a
/// whole-track upload rather than after one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioScope {
    /// Text or JSON only.
    None,
    /// Named excerpts only.
    Excerpts,
    /// The complete track.
    WholeTrack,
}

impl AudioScope {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Excerpts => "excerpts",
            Self::WholeTrack => "whole-track",
        }
    }
}

/// One (model, contract) verdict and the evidence behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuitabilityRow {
    /// The model identity: an OpenRouter slug, or a local runtime's model id.
    pub model_id: &'static str,
    pub contract: ContractId,
    pub status: Suitability,
    /// ISO date of the measurement. Required for `recommended`.
    pub evidence_date: Option<&'static str>,
    /// Where the measurement is written down.
    pub evidence: &'static str,
    /// The prompt or localization-policy version that was benchmarked.
    /// Required for `recommended`.
    pub prompt_version: Option<&'static str>,
    /// The output schema the benchmarked run validated against. Required for
    /// `recommended`.
    pub schema_version: Option<&'static str>,
    pub audio_scope: AudioScope,
    /// BCP-47-ish tags, or a description of the alphabet for a romanizing
    /// aligner. `["*"]` means "no language restriction measured".
    pub languages: &'static [&'static str],
    /// What the measurement found wrong, in the words of the evidence.
    pub limitations: &'static [&'static str],
}

/// The table.
///
/// `#[rustfmt::skip]` for the repository-guide reason: this is checked row by
/// row against `docs/LYRICS_TIMING_BENCHMARK_RESULTS.md`, and one field per
/// line stops being checkable.
#[rustfmt::skip]
pub const OVERLAY: &[SuitabilityRow] = &[
    // The 2026-08-04 four-track benchmark: 100 % authored-line coverage on all
    // four tracks (15/15, 33/33, 51/51, 42/42), the canary's two Whisper-looped
    // outro lines recovered, and operator listen-check adjudication of 21 lines
    // finding the anchor lane correct on 14 against the baseline's 3. It is the
    // production default since tranche LT1.
    SuitabilityRow {
        model_id: "mms-ctc",
        contract: ContractId::Align,
        status: Suitability::Recommended,
        evidence_date: Some("2026-08-04"),
        evidence: "docs/LYRICS_TIMING_BENCHMARK_RESULTS.md: four-track benchmark plus operator adjudication",
        prompt_version: Some("anchor-block-mms/1"),
        schema_version: Some("musializer.lyric-timing/v1"),
        audio_scope: AudioScope::WholeTrack,
        languages: &["romanized text normalized to the MMS_FA alphabet"],
        limitations: &[
            "the aligner's own score does not separate right from wrong (median 0.139 correct vs 0.142 wrong); flag cross-lane disagreement instead",
            "repeated chorus phrases can select the wrong occurrence; the correct response is the unresolved state",
            "short one-word exclamations with weak acoustic evidence can drift unflagged (shipped-the-disposition line 41, 134.6 s against 156.2 s)",
        ],
    },
    // Failed its decision gate on the same day and on the same four tracks.
    // Retired as a lane; the harness is kept. Never offered, at any
    // show_experimental setting.
    SuitabilityRow {
        model_id: "qwen3-fa",
        contract: ContractId::Align,
        status: Suitability::Unsupported,
        evidence_date: Some("2026-08-04"),
        evidence: "docs/LYRICS_TIMING_BENCHMARK_RESULTS.md: Qwen3-ForcedAligner fails its gate",
        prompt_version: None,
        schema_version: Some("musializer.lyric-timing/v1"),
        audio_scope: AudioScope::WholeTrack,
        languages: &["*"],
        limitations: &[
            "coverage collapses on real songs: 47-76 % on the three longer tracks",
            "catastrophic mid-song drift, median 50 s on the choir track",
            "validated only on speech, which the web evidence predicted",
        ],
    },
    // In production use for MiMo feelings and never benchmarked. It is here so
    // the absence of evidence is stated rather than inferred; without a row it
    // would resolve to experimental anyway, and this says why.
    SuitabilityRow {
        model_id: "xiaomi/mimo-v2.5",
        contract: ContractId::Semantic,
        status: Suitability::Experimental,
        evidence_date: None,
        evidence: "in use by tools/mimo_openrouter.py; no benchmark has been run",
        prompt_version: None,
        schema_version: None,
        audio_scope: AudioScope::WholeTrack,
        languages: &["*"],
        limitations: &[
            "never benchmarked: no measured evidence for or against it",
            "sends the complete track, so TC-SEMANTIC's audio-leaves-machine confirmation applies per job",
        ],
    },
];

/// The row for a (model, contract) pair, when there is one.
#[must_use]
pub fn row(model_id: &str, contract: ContractId) -> Option<&'static SuitabilityRow> {
    OVERLAY
        .iter()
        .find(|row| row.model_id == model_id && row.contract == contract)
}

/// The verdict for a (model, contract) pair.
///
/// **An absent pair is [`Suitability::Experimental`]**, which is the whole
/// lookup rule: an unmeasured model is neither blessed nor banned, so it is
/// available to a user who asks for it and invisible to one who does not.
#[must_use]
pub fn status(model_id: &str, contract: ContractId) -> Suitability {
    row(model_id, contract).map_or(Suitability::Experimental, |row| row.status)
}

/// Every row for a contract, in table order. What a picker groups by.
pub fn rows_for(contract: ContractId) -> impl Iterator<Item = &'static SuitabilityRow> {
    OVERLAY.iter().filter(move |row| row.contract == contract)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::contracts::{Boundary, ALL_CONTRACTS};
    use std::collections::BTreeSet;

    #[test]
    fn a_model_absent_from_the_table_is_experimental_never_recommended() {
        for contract in ALL_CONTRACTS {
            let verdict = status("acme/never-measured", contract);
            assert_eq!(verdict, Suitability::Experimental);
            assert_ne!(verdict, Suitability::Recommended);
            assert!(row("acme/never-measured", contract).is_none());
        }
        // Including a model that *is* in the table, under a contract it was
        // never measured for. The key is the pair, not the model.
        assert_eq!(
            status("mms-ctc", ContractId::Semantic),
            Suitability::Experimental
        );
        assert_eq!(
            status("xiaomi/mimo-v2.5", ContractId::Align),
            Suitability::Experimental
        );
    }

    #[test]
    fn every_recommended_row_carries_an_evidence_date_and_a_benchmarked_version() {
        let mut recommended = 0;
        for row in OVERLAY {
            if row.status != Suitability::Recommended {
                continue;
            }
            recommended += 1;
            let named = format!("{} / {}", row.model_id, row.contract.token());
            assert!(
                row.evidence_date.is_some_and(|date| date.len() == 10),
                "{named} is recommended without an ISO evidence date"
            );
            assert!(
                row.prompt_version.is_some_and(|value| !value.is_empty()),
                "{named} is recommended without a benchmarked prompt version"
            );
            assert!(
                row.schema_version.is_some_and(|value| !value.is_empty()),
                "{named} is recommended without a benchmarked schema version"
            );
            assert!(
                !row.evidence.is_empty(),
                "{named} is recommended without naming where the measurement is written down"
            );
        }
        assert!(recommended > 0, "the table has no recommended row to check");
    }

    #[test]
    fn the_key_is_unique() {
        let mut seen = BTreeSet::new();
        for row in OVERLAY {
            assert!(
                seen.insert((row.model_id, row.contract)),
                "{} / {} appears twice",
                row.model_id,
                row.contract.token()
            );
        }
        assert_eq!(seen.len(), OVERLAY.len());
    }

    /// The seeded rows, pinned. A future row is a deliberate edit here, which is
    /// the point: evidence is added by measuring, not by widening a filter.
    #[test]
    fn the_table_is_exactly_this_repositorys_evidence() {
        let seeded: Vec<(&str, ContractId, Suitability, Option<&str>)> = OVERLAY
            .iter()
            .map(|row| (row.model_id, row.contract, row.status, row.evidence_date))
            .collect();
        assert_eq!(
            seeded,
            vec![
                (
                    "mms-ctc",
                    ContractId::Align,
                    Suitability::Recommended,
                    Some("2026-08-04")
                ),
                (
                    "qwen3-fa",
                    ContractId::Align,
                    Suitability::Unsupported,
                    Some("2026-08-04")
                ),
                (
                    "xiaomi/mimo-v2.5",
                    ContractId::Semantic,
                    Suitability::Experimental,
                    None
                ),
            ]
        );
    }

    #[test]
    fn every_row_names_a_limitation_and_its_evidence() {
        for row in OVERLAY {
            assert!(
                !row.limitations.is_empty(),
                "{} / {} claims no limitations at all",
                row.model_id,
                row.contract.token()
            );
            assert!(!row.evidence.is_empty());
            assert!(!row.languages.is_empty());
        }
    }

    #[test]
    fn an_unsupported_row_is_never_offered_and_experimental_needs_asking() {
        assert!(Suitability::Recommended.offered(false));
        assert!(Suitability::Recommended.offered(true));
        assert!(!Suitability::Experimental.offered(false));
        assert!(Suitability::Experimental.offered(true));
        assert!(!Suitability::Unsupported.offered(false));
        assert!(!Suitability::Unsupported.offered(true));
        assert!(!status("qwen3-fa", ContractId::Align).offered(true));
    }

    #[test]
    fn the_align_rows_stay_inside_the_contracts_own_boundary() {
        // Both aligners take the whole track, and TC-ALIGN is local-only by
        // contract, so "whole-track" here can never mean "uploaded".
        assert_eq!(ContractId::Align.max_boundary(), Boundary::LocalOnly);
        for row in rows_for(ContractId::Align) {
            assert_eq!(row.audio_scope, AudioScope::WholeTrack);
        }
        assert_eq!(rows_for(ContractId::Align).count(), 2);
        assert_eq!(rows_for(ContractId::Wording).count(), 0);
    }

    #[test]
    fn tokens_round_trip_and_the_revision_is_dated() {
        for status in [
            Suitability::Recommended,
            Suitability::Experimental,
            Suitability::Unsupported,
        ] {
            assert_eq!(Suitability::parse(status.token()), Some(status));
            assert_eq!(
                serde_json::to_string(&status).unwrap(),
                format!("\"{}\"", status.token())
            );
        }
        assert_eq!(Suitability::parse("preferred"), None);
        for scope in [
            AudioScope::None,
            AudioScope::Excerpts,
            AudioScope::WholeTrack,
        ] {
            assert_eq!(
                serde_json::to_string(&scope).unwrap(),
                format!("\"{}\"", scope.token())
            );
        }
        assert!(OVERLAY_REVISION.starts_with("musializer.assist-suitability/"));
    }
}
