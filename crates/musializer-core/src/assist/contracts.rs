//! The task-contract table and the boundary ladder.
//!
//! `docs/ASSIST_PROVIDER_CONTRACTS.md` §1 and §5. The table is the whole point:
//! a route may never be resolved for a contract whose declared maximum boundary
//! is lower than the route's own, and a fallback policy may never raise the
//! rank of the data leaving the machine.
//!
//! The tables carry `#[rustfmt::skip]` for the reason the repository guide
//! gives: they are checked row-by-row against the design document, and one
//! argument per line stops being checkable.

use serde::{Deserialize, Serialize};

/// How far the data for a contract may travel. §1's ladder, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Boundary {
    /// Nothing leaves the machine; no socket is opened.
    LocalOnly,
    /// Derived text/JSON leaves; no audio, no raw PCM.
    TextLeavesMachine,
    /// Audio bytes leave; requires per-job confirmation.
    AudioLeavesMachine,
}

impl Boundary {
    /// The rank in §1's ladder. `Ord` agrees with this by declaration order,
    /// and the ranks are what §5's invariants are written in terms of.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::LocalOnly => 0,
            Self::TextLeavesMachine => 1,
            Self::AudioLeavesMachine => 2,
        }
    }

    /// The stable token used in files and in the execution snapshot.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::LocalOnly => "local-only",
            Self::TextLeavesMachine => "text-leaves-machine",
            Self::AudioLeavesMachine => "audio-leaves-machine",
        }
    }

    /// Parses a token. Unknown tokens are refused rather than defaulted, because
    /// a defaulted boundary would be a silent widening.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        [
            Self::LocalOnly,
            Self::TextLeavesMachine,
            Self::AudioLeavesMachine,
        ]
        .into_iter()
        .find(|candidate| candidate.token() == token)
    }
}

/// How a contract is executed. §1's route types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouteType {
    /// In-process deterministic Rust.
    Builtin,
    /// A child process on this machine.
    LocalProc,
    /// The installed `codex exec`.
    Codex,
    /// The OpenRouter HTTP API. Spelled as one word, not `open-router`, because
    /// it is the provider's own name and the token reaches a file.
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl RouteType {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::LocalProc => "local-proc",
            Self::Codex => "codex",
            Self::OpenRouter => "openrouter",
        }
    }

    /// The lowest boundary this route type can possibly operate at. A `builtin`
    /// or `local-proc` route opens no socket by construction; the other two
    /// always do.
    #[must_use]
    pub const fn minimum_boundary(self) -> Boundary {
        match self {
            Self::Builtin | Self::LocalProc => Boundary::LocalOnly,
            Self::Codex | Self::OpenRouter => Boundary::TextLeavesMachine,
        }
    }
}

/// What happens when a route fails. §5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackPolicy {
    /// Fail the contract and report it. No substitution.
    None,
    /// Pause the job and wait for a decision.
    Ask,
    /// Substitute only routes with boundary rank 0.
    LocalOnly,
    /// Substitute only routes whose boundary rank is **equal** to the failed
    /// route's. Not "≤": a quiet downgrade also changes which model produced
    /// the result, so it is a decision rather than a default.
    SameBoundary,
}

impl FallbackPolicy {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ask => "ask",
            Self::LocalOnly => "local-only",
            Self::SameBoundary => "same-boundary",
        }
    }

    /// §5 invariant 1: a local failure never becomes a remote request without a
    /// new decision. Only [`FallbackPolicy::Ask`] can raise the rank, and even
    /// then only through an explicit answer, which is why `Ask` answers `false`
    /// here for a raise — this function is the *automatic* substitution rule.
    #[must_use]
    pub fn permits_automatic_substitute(self, failed: Boundary, candidate: Boundary) -> bool {
        match self {
            Self::None | Self::Ask => false,
            Self::LocalOnly => candidate.rank() == 0,
            Self::SameBoundary => candidate.rank() == failed.rank(),
        }
    }
}

/// A task contract. §1's stable `TC-*` tokens, which are the key in preferences,
/// in the execution snapshot, and in cache acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContractId {
    #[serde(rename = "TC-MEASURED")]
    Measured,
    #[serde(rename = "TC-COARSE")]
    Coarse,
    #[serde(rename = "TC-ALIGN")]
    Align,
    #[serde(rename = "TC-WORDING")]
    Wording,
    #[serde(rename = "TC-SEMANTIC")]
    Semantic,
    #[serde(rename = "TC-PLAN")]
    Plan,
    #[serde(rename = "TC-VERIFY")]
    Verify,
}

/// Every contract, in the document's order.
pub const ALL_CONTRACTS: [ContractId; 7] = [
    ContractId::Measured,
    ContractId::Coarse,
    ContractId::Align,
    ContractId::Wording,
    ContractId::Semantic,
    ContractId::Plan,
    ContractId::Verify,
];

#[rustfmt::skip]
impl ContractId {
    /// The stable token. Checked column-by-column against §1's table.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Measured => "TC-MEASURED",
            Self::Coarse   => "TC-COARSE",
            Self::Align    => "TC-ALIGN",
            Self::Wording  => "TC-WORDING",
            Self::Semantic => "TC-SEMANTIC",
            Self::Plan     => "TC-PLAN",
            Self::Verify   => "TC-VERIFY",
        }
    }

    /// The highest boundary a route for this contract may reach.
    ///
    /// `TC-ALIGN` is `local-only` by contract even though remote aligners
    /// exist: raising it needs a benchmark and a schema bump, not a new route
    /// entry (§1).
    #[must_use]
    pub const fn max_boundary(self) -> Boundary {
        match self {
            Self::Measured => Boundary::LocalOnly,
            Self::Coarse   => Boundary::AudioLeavesMachine,
            Self::Align    => Boundary::LocalOnly,
            Self::Wording  => Boundary::TextLeavesMachine,
            Self::Semantic => Boundary::AudioLeavesMachine,
            Self::Plan     => Boundary::TextLeavesMachine,
            Self::Verify   => Boundary::AudioLeavesMachine,
        }
    }

    /// Which route types may serve this contract.
    #[must_use]
    pub const fn eligible_route_types(self) -> &'static [RouteType] {
        match self {
            Self::Measured => &[RouteType::Builtin],
            Self::Coarse   => &[RouteType::LocalProc, RouteType::OpenRouter],
            Self::Align    => &[RouteType::LocalProc],
            Self::Wording  => &[RouteType::Codex, RouteType::OpenRouter],
            Self::Semantic => &[RouteType::OpenRouter],
            Self::Plan     => &[RouteType::Builtin, RouteType::Codex, RouteType::OpenRouter],
            Self::Verify   => &[RouteType::LocalProc, RouteType::OpenRouter],
        }
    }

    /// Which fallback policies may be stored for this contract.
    #[must_use]
    pub const fn allowed_fallbacks(self) -> &'static [FallbackPolicy] {
        match self {
            Self::Measured => &[FallbackPolicy::None],
            Self::Coarse   => &[FallbackPolicy::None, FallbackPolicy::Ask, FallbackPolicy::LocalOnly, FallbackPolicy::SameBoundary],
            Self::Align    => &[FallbackPolicy::None, FallbackPolicy::LocalOnly],
            Self::Wording  => &[FallbackPolicy::None, FallbackPolicy::Ask, FallbackPolicy::SameBoundary],
            Self::Semantic => &[FallbackPolicy::None, FallbackPolicy::Ask],
            Self::Plan     => &[FallbackPolicy::None, FallbackPolicy::Ask, FallbackPolicy::LocalOnly, FallbackPolicy::SameBoundary],
            Self::Verify   => &[FallbackPolicy::None, FallbackPolicy::Ask, FallbackPolicy::LocalOnly, FallbackPolicy::SameBoundary],
        }
    }
}

impl ContractId {
    /// Parses a `TC-*` token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        ALL_CONTRACTS
            .into_iter()
            .find(|candidate| candidate.token() == token)
    }

    /// `TC-MEASURED` appears in the routing matrix so the user can see the whole
    /// pipeline, but it is not user-routable (§7 decision 2).
    #[must_use]
    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Measured)
    }

    /// Whether this contract's data can leave the machine at all. §2's default
    /// for `allow_fallbacks` and `zdr_required` keys off it.
    #[must_use]
    pub const fn leaves_machine(self) -> bool {
        self.max_boundary().rank() >= 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_token_round_trips() {
        for contract in ALL_CONTRACTS {
            assert_eq!(ContractId::parse(contract.token()), Some(contract));
        }
        assert_eq!(ContractId::parse("TC-NOPE"), None);
        assert_eq!(ContractId::parse("tc-measured"), None);
    }

    #[test]
    fn every_boundary_token_round_trips() {
        for boundary in [
            Boundary::LocalOnly,
            Boundary::TextLeavesMachine,
            Boundary::AudioLeavesMachine,
        ] {
            assert_eq!(Boundary::parse(boundary.token()), Some(boundary));
        }
        assert_eq!(Boundary::parse("offline"), None);
    }

    #[test]
    fn boundary_rank_matches_declaration_order() {
        assert_eq!(Boundary::LocalOnly.rank(), 0);
        assert_eq!(Boundary::TextLeavesMachine.rank(), 1);
        assert_eq!(Boundary::AudioLeavesMachine.rank(), 2);
        assert!(Boundary::LocalOnly < Boundary::TextLeavesMachine);
        assert!(Boundary::TextLeavesMachine < Boundary::AudioLeavesMachine);
    }

    #[test]
    fn no_eligible_route_type_can_exceed_its_contract_boundary() {
        for contract in ALL_CONTRACTS {
            for route_type in contract.eligible_route_types() {
                assert!(
                    route_type.minimum_boundary().rank() <= contract.max_boundary().rank(),
                    "{} allows {} but caps at {}",
                    contract.token(),
                    route_type.token(),
                    contract.max_boundary().token(),
                );
            }
        }
    }

    #[test]
    fn locked_contract_is_builtin_local_and_unfallbackable() {
        let measured = ContractId::Measured;
        assert!(measured.is_locked());
        assert_eq!(measured.max_boundary(), Boundary::LocalOnly);
        assert_eq!(measured.eligible_route_types(), &[RouteType::Builtin]);
        assert_eq!(measured.allowed_fallbacks(), &[FallbackPolicy::None]);
        for contract in ALL_CONTRACTS {
            assert_eq!(contract.is_locked(), contract == ContractId::Measured);
        }
    }

    #[test]
    fn align_stays_local_only_however_it_is_asked() {
        assert_eq!(ContractId::Align.max_boundary(), Boundary::LocalOnly);
        assert!(!ContractId::Align
            .allowed_fallbacks()
            .contains(&FallbackPolicy::SameBoundary));
        assert!(!ContractId::Align
            .allowed_fallbacks()
            .contains(&FallbackPolicy::Ask));
    }

    #[test]
    fn automatic_substitution_never_raises_the_boundary_rank() {
        let ladder = [
            Boundary::LocalOnly,
            Boundary::TextLeavesMachine,
            Boundary::AudioLeavesMachine,
        ];
        for policy in [
            FallbackPolicy::None,
            FallbackPolicy::Ask,
            FallbackPolicy::LocalOnly,
            FallbackPolicy::SameBoundary,
        ] {
            for failed in ladder {
                for candidate in ladder {
                    if policy.permits_automatic_substitute(failed, candidate) {
                        assert!(
                            candidate.rank() <= failed.rank(),
                            "{} raised {} to {}",
                            policy.token(),
                            failed.token(),
                            candidate.token(),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn same_boundary_is_equal_rank_not_at_most() {
        let policy = FallbackPolicy::SameBoundary;
        assert!(policy.permits_automatic_substitute(
            Boundary::AudioLeavesMachine,
            Boundary::AudioLeavesMachine
        ));
        assert!(
            !policy.permits_automatic_substitute(Boundary::AudioLeavesMachine, Boundary::LocalOnly)
        );
        assert!(!policy.permits_automatic_substitute(
            Boundary::AudioLeavesMachine,
            Boundary::TextLeavesMachine
        ));
    }

    #[test]
    fn ask_never_substitutes_automatically() {
        for failed in [Boundary::LocalOnly, Boundary::AudioLeavesMachine] {
            for candidate in [Boundary::LocalOnly, Boundary::AudioLeavesMachine] {
                assert!(!FallbackPolicy::Ask.permits_automatic_substitute(failed, candidate));
                assert!(!FallbackPolicy::None.permits_automatic_substitute(failed, candidate));
            }
        }
    }

    #[test]
    fn contract_tokens_serialize_as_the_stable_spelling() {
        for contract in ALL_CONTRACTS {
            let json = serde_json::to_string(&contract).unwrap();
            assert_eq!(json, format!("\"{}\"", contract.token()));
            let parsed: ContractId = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, contract);
        }
    }

    #[test]
    fn enum_tokens_serialize_as_their_documented_spelling() {
        for route_type in [
            RouteType::Builtin,
            RouteType::LocalProc,
            RouteType::Codex,
            RouteType::OpenRouter,
        ] {
            assert_eq!(
                serde_json::to_string(&route_type).unwrap(),
                format!("\"{}\"", route_type.token())
            );
        }
        for policy in [
            FallbackPolicy::None,
            FallbackPolicy::Ask,
            FallbackPolicy::LocalOnly,
            FallbackPolicy::SameBoundary,
        ] {
            assert_eq!(
                serde_json::to_string(&policy).unwrap(),
                format!("\"{}\"", policy.token())
            );
        }
        for boundary in [
            Boundary::LocalOnly,
            Boundary::TextLeavesMachine,
            Boundary::AudioLeavesMachine,
        ] {
            assert_eq!(
                serde_json::to_string(&boundary).unwrap(),
                format!("\"{}\"", boundary.token())
            );
        }
    }
}
