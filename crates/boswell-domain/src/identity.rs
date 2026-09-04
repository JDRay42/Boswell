//! Identity as a port (`IdentityProvider` / "IAuth") — procedural memory, Phase 4.
//!
//! Every trust claim on the write path rests on authenticated, unforgeable agent
//! identity and a verifiable delegation chain — which Boswell must not hardcode to
//! any one system. Following the existing port pattern ([`crate::traits::ClaimStore`],
//! [`crate::traits::LlmProvider`]), identity is an adapter behind a small, stable
//! trait (design §6).
//!
//! **The boundary is deliberate.** An [`IdentityProvider`] *authenticates and
//! attests* — who you are, whether the delegation chain is real, and at what
//! [`Assurance`]. Boswell *authorizes* — what a principal may write (namespace,
//! `max_tier`, ops), via an [`AuthorizationPolicy`]. Authorization is domain
//! policy and stays inside Boswell; an external identity system never owns it.
//!
//! With no backend, [`NullIdentityProvider`] yields `Assurance::None`, whose tier
//! ceiling is `Ephemeral` — so Boswell still runs as a local, single-principal,
//! ephemeral-tier memory and nothing self-asserted reaches shared tiers. Plugging
//! in an attested provider later makes the *same* writes eligible to climb, with
//! no change to the ceiling formula ([`crate::entry_tier`]).

use crate::write_path::{Assurance, Authority};
use crate::DelegationChain;

/// What kind of principal an identity denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    /// A human.
    Human,
    /// An automated agent.
    Agent,
    /// A non-agent service.
    Service,
}

impl PrincipalKind {
    /// Stable string form.
    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalKind::Human => "human",
            PrincipalKind::Agent => "agent",
            PrincipalKind::Service => "service",
        }
    }

    /// Parse from the string form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "human" => Some(PrincipalKind::Human),
            "agent" => Some(PrincipalKind::Agent),
            "service" => Some(PrincipalKind::Service),
            _ => None,
        }
    }
}

/// An authenticated principal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Stable identity, e.g. `agent:worker`.
    pub id: String,
    /// The kind of principal.
    pub kind: PrincipalKind,
}

impl Principal {
    /// Construct a principal.
    pub fn new(id: impl Into<String>, kind: PrincipalKind) -> Self {
        Self {
            id: id.into(),
            kind,
        }
    }
}

/// A credential presented for authentication. Opaque to the domain; each
/// [`IdentityProvider`] interprets its own scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The credential token/secret (interpretation is provider-specific).
    pub token: String,
}

impl Credential {
    /// Wrap a token as a credential.
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

/// Why authentication failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The credential did not match any known principal.
    UnknownCredential,
    /// The credential was malformed.
    MalformedCredential(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::UnknownCredential => write!(f, "unknown credential"),
            AuthError::MalformedCredential(m) => write!(f, "malformed credential: {}", m),
        }
    }
}

impl std::error::Error for AuthError {}

/// The result of verifying a delegation chain: whether it is real, and the
/// [`Assurance`] the provider attests for it (design §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationVerdict {
    /// Whether the delegation chain is valid.
    pub valid: bool,
    /// The identity assurance attested for the chain.
    pub assurance: Assurance,
}

/// Authenticates principals and attests delegation chains (design §6).
///
/// This port never authorizes — see [`AuthorizationPolicy`].
pub trait IdentityProvider {
    /// Authenticate a credential into a [`Principal`].
    fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError>;

    /// Verify a delegation chain, returning validity and attested assurance.
    fn verify_delegation(&self, chain: &DelegationChain) -> DelegationVerdict;
}

/// Maps an authenticated [`Principal`] to what it may do in Boswell (design §6).
///
/// This is Boswell's authorization policy — domain logic, kept inside Boswell and
/// never delegated to the identity system.
pub trait AuthorizationPolicy {
    /// The [`Authority`] granted to `principal`, or `None` if it has none.
    fn authority_for(&self, principal: &Principal) -> Option<Authority>;
}

/// The no-backend identity provider: a single local principal, `Assurance::None`.
///
/// Because `Assurance::None`'s tier ceiling is `Ephemeral`, everything it authors
/// stays ephemeral no matter how broad its [`Authority`] — the "safe by
/// construction" local default (design §6).
#[derive(Debug, Clone, Default)]
pub struct NullIdentityProvider;

impl NullIdentityProvider {
    /// The id of the single local principal.
    pub const LOCAL_ID: &'static str = "local";
}

impl IdentityProvider for NullIdentityProvider {
    fn authenticate(&self, _credential: &Credential) -> Result<Principal, AuthError> {
        Ok(Principal::new(Self::LOCAL_ID, PrincipalKind::Human))
    }

    fn verify_delegation(&self, _chain: &DelegationChain) -> DelegationVerdict {
        // Self-asserted at best: nothing reaches shared/long-term tiers.
        DelegationVerdict {
            valid: true,
            assurance: Assurance::None,
        }
    }
}

/// The authorization policy paired with [`NullIdentityProvider`]: the local
/// principal may write anywhere at any tier, but its `Assurance::None` ceiling
/// keeps every entry ephemeral in practice. Any other principal gets nothing.
#[derive(Debug, Clone, Default)]
pub struct LocalAuthorizationPolicy;

impl AuthorizationPolicy for LocalAuthorizationPolicy {
    fn authority_for(&self, principal: &Principal) -> Option<Authority> {
        use crate::write_path::Op;
        use crate::Tier;
        if principal.id == NullIdentityProvider::LOCAL_ID {
            Some(Authority {
                namespaces: vec!["*".to_string()],
                max_tier: Tier::Permanent,
                ops: vec![Op::Read, Op::Write],
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{entry_tier, EvidenceType, ProvenanceStamp, Tier};

    #[test]
    fn principal_kind_roundtrips() {
        for k in [
            PrincipalKind::Human,
            PrincipalKind::Agent,
            PrincipalKind::Service,
        ] {
            assert_eq!(PrincipalKind::parse(k.as_str()), Some(k));
        }
        assert!(PrincipalKind::parse("robot").is_none());
    }

    #[test]
    fn null_provider_is_single_principal_ephemeral() {
        let provider = NullIdentityProvider;
        let principal = provider.authenticate(&Credential::new("anything")).unwrap();
        assert_eq!(principal.id, NullIdentityProvider::LOCAL_ID);

        let verdict = provider.verify_delegation(&DelegationChain::default());
        assert!(verdict.valid);
        assert_eq!(verdict.assurance, Assurance::None);

        // Even with permanent authority, the None-assurance ceiling caps entry at
        // ephemeral — the safe-by-construction local default.
        let authority = LocalAuthorizationPolicy
            .authority_for(&principal)
            .expect("local principal is authorized");
        let stamp = ProvenanceStamp {
            author: principal.id.clone(),
            delegation_chain: DelegationChain(vec![principal.id.clone()]),
            authority,
            evidence: EvidenceType::Observed,
            assurance: verdict.assurance,
            task_id: None,
            session_id: None,
            timestamp: 1,
            dev_provider: false,
        };
        assert_eq!(entry_tier(Tier::Permanent, &stamp), Tier::Ephemeral);
    }

    #[test]
    fn local_policy_denies_unknown_principals() {
        assert!(LocalAuthorizationPolicy
            .authority_for(&Principal::new("agent:stranger", PrincipalKind::Agent))
            .is_none());
    }
}
