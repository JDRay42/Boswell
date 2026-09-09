//! # boswell-devauth — a development-only identity adapter
//!
//! **DEVELOPMENT / TESTING ONLY.** This crate implements a fake
//! [`IdentityProvider`] so anyone can stand Boswell up and exercise the whole
//! trust gradient (design §7) without a real identity system. It must never be
//! trusted for long-term memory, and it is deliberately **not** a dependency of
//! any production binary, so production builds exclude it entirely.
//!
//! It is loud by design (design §7.2):
//! - **Refuses to run without explicit opt-in** — [`DevAuth::new`] returns
//!   [`DevAuthError::NotOptedIn`] unless `allow_dev_auth` is set.
//! - **Hard production lockout** — [`DevAuth::new`] returns
//!   [`DevAuthError::ProductionLockout`] when a production environment is signaled,
//!   even if opt-in is set.
//! - **Persistent warnings** — a startup banner ([`DevAuth::banner`]) and a
//!   `tracing::warn!` on construction and on every principal assignment / stamp.
//! - **Downstream marker** — [`DEV_AUTH_MARKER`] for an `X-Boswell-Auth` header or
//!   a `warnings[]` field (wiring into the gateway is left to that layer).
//! - **Provenance tainting** — every [`DevAuth::stamp`] sets `dev_provider = true`
//!   so dev-authored entries stay distinguishable and sweepable.
//! - **Store isolation** — operators should point devAuth at a throwaway DB; this
//!   crate does not touch storage, keeping that choice with the operator.
//!
//! The four sample identities ([`DevIdentity`]) have differentiated authority so
//! the gradient is observable end to end: the interloper stays stuck at ephemeral,
//! the worker writes task-tier, the project-leader can endorse into project tier,
//! and the memory-manager curates.

#![warn(missing_docs)]

use boswell_domain::{
    Assurance, AuthError, Authority, AuthorizationPolicy, Credential, DelegationChain,
    DelegationVerdict, EvidenceType, IdentityProvider, Op, Principal, PrincipalKind,
    ProvenanceStamp, Tier,
};
use thiserror::Error;

/// The downstream marker value (e.g. for an `X-Boswell-Auth` header) identifying a
/// response served under a devAuth principal.
pub const DEV_AUTH_MARKER: &str = "dev-untrusted";

/// The warning attached to every devAuth action.
pub const DEV_AUTH_WARNING: &str = "boswell-devauth is DEVELOPMENT/TESTING ONLY \
    — these identities must not be trusted for long-term memory";

/// A loud startup banner for operators.
pub const DEV_AUTH_BANNER: &str = "\
============================================================
  BOSWELL devAuth ENABLED — DEVELOPMENT / TESTING ONLY
  Fake identities. Do NOT use in production or trust for
  long-term memory. Every write is tainted dev_provider=true.
============================================================";

/// Why devAuth refused to start (design §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DevAuthError {
    /// devAuth was selected without explicit opt-in.
    #[error("boswell-devauth requires explicit opt-in (allow_dev_auth = true); refusing to start")]
    NotOptedIn,
    /// devAuth was selected in a production environment.
    #[error("boswell-devauth must never run in a production environment; refusing to start")]
    ProductionLockout,
    /// devAuth was selected without the environment being declared at all.
    #[error(
        "boswell-devauth requires the environment to be declared non-production \
         (set BOSWELL_ENV, e.g. BOSWELL_ENV=development); refusing to start"
    )]
    EnvironmentUndeclared,
}

/// Configuration gating whether devAuth may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DevAuthConfig {
    /// The operator's explicit opt-in. Without this, [`DevAuth::new`] refuses.
    pub allow_dev_auth: bool,
    /// Whether a production environment/flag is set. If so, [`DevAuth::new`] is a
    /// fatal error regardless of `allow_dev_auth`.
    pub production: bool,
    /// Whether the environment was declared at all. An *undeclared* environment
    /// is treated as production (see [`DevAuth::new`]).
    pub environment_declared: bool,
}

impl DevAuthConfig {
    /// Read the config from the environment: `allow_dev_auth` from
    /// `BOSWELL_ALLOW_DEV_AUTH` (truthy), and the environment from `BOSWELL_ENV`.
    ///
    /// An unset or blank `BOSWELL_ENV` leaves `environment_declared` false, which
    /// fails closed — see [`DevAuth::new`].
    pub fn from_env() -> Self {
        let truthy =
            |v: String| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        let env = std::env::var("BOSWELL_ENV").unwrap_or_default();
        let env = env.trim();
        Self {
            allow_dev_auth: std::env::var("BOSWELL_ALLOW_DEV_AUTH")
                .map(truthy)
                .unwrap_or(false),
            production: env.eq_ignore_ascii_case("production"),
            environment_declared: !env.is_empty(),
        }
    }
}

/// The four fixed sample identities (design §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevIdentity {
    /// The ordinary agent: writes task-tier, advocates upward.
    StandardWorker,
    /// The red-team identity: writes can't climb; evidence forced weak.
    UntrustedInterloper,
    /// Endorses a worker's advocated entry so it can climb to project tier.
    ProjectLeader,
    /// The curator: promote/demote/forget/GC, resolve contradictions.
    MemoryManager,
}

impl DevIdentity {
    /// All four identities.
    pub fn all() -> [DevIdentity; 4] {
        [
            DevIdentity::StandardWorker,
            DevIdentity::UntrustedInterloper,
            DevIdentity::ProjectLeader,
            DevIdentity::MemoryManager,
        ]
    }

    /// The slug used as the credential token, e.g. `"standard-worker"`.
    pub fn name(&self) -> &'static str {
        match self {
            DevIdentity::StandardWorker => "standard-worker",
            DevIdentity::UntrustedInterloper => "untrusted-interloper",
            DevIdentity::ProjectLeader => "project-leader",
            DevIdentity::MemoryManager => "memory-manager",
        }
    }

    /// Parse an identity from its slug.
    pub fn parse(s: &str) -> Option<Self> {
        Self::all().into_iter().find(|i| i.name() == s)
    }

    /// The stable principal id this identity authenticates as.
    pub fn principal_id(&self) -> &'static str {
        match self {
            DevIdentity::StandardWorker => "agent:worker",
            DevIdentity::UntrustedInterloper => "agent:interloper",
            DevIdentity::ProjectLeader => "project:lead",
            DevIdentity::MemoryManager => "service:memory-manager",
        }
    }

    /// Look up an identity by its principal id.
    pub fn by_principal_id(id: &str) -> Option<Self> {
        Self::all().into_iter().find(|i| i.principal_id() == id)
    }

    /// The authenticated principal.
    pub fn principal(&self) -> Principal {
        let kind = match self {
            DevIdentity::MemoryManager => PrincipalKind::Service,
            _ => PrincipalKind::Agent,
        };
        Principal::new(self.principal_id(), kind)
    }

    /// The nominal identity assurance (design §7.1).
    pub fn nominal_assurance(&self) -> Assurance {
        match self {
            DevIdentity::StandardWorker => Assurance::Verified,
            DevIdentity::UntrustedInterloper => Assurance::Asserted,
            DevIdentity::ProjectLeader => Assurance::Attested,
            DevIdentity::MemoryManager => Assurance::Attested,
        }
    }

    /// The Boswell-side [`Authority`] granted to this identity (design §7.1).
    pub fn authority(&self) -> Authority {
        match self {
            DevIdentity::StandardWorker => Authority {
                namespaces: vec!["agent:worker".into()],
                max_tier: Tier::Task,
                ops: vec![Op::Read, Op::Write],
            },
            DevIdentity::UntrustedInterloper => Authority {
                namespaces: vec!["agent:interloper".into()],
                max_tier: Tier::Ephemeral,
                ops: vec![Op::Write],
            },
            DevIdentity::ProjectLeader => Authority {
                namespaces: vec!["project".into()],
                max_tier: Tier::Project,
                ops: vec![Op::Read, Op::Write, Op::Endorse],
            },
            DevIdentity::MemoryManager => Authority {
                namespaces: vec!["*".into()],
                max_tier: Tier::Permanent,
                ops: vec![Op::Read, Op::Write, Op::Curate],
            },
        }
    }

    /// Coerce the evidence type this identity is allowed to assert. The interloper
    /// is forced to weak evidence (`tool_output`/`reported`) so its writes cannot
    /// reach team tier on their own (design §7.1); others pass through unchanged.
    pub fn coerce_evidence(&self, requested: EvidenceType) -> EvidenceType {
        match self {
            DevIdentity::UntrustedInterloper => match requested {
                EvidenceType::Reported | EvidenceType::ToolOutput => requested,
                _ => EvidenceType::ToolOutput,
            },
            _ => requested,
        }
    }
}

/// The development identity adapter. Stateless; construct with [`DevAuth::new`].
#[derive(Debug, Clone, Copy)]
pub struct DevAuth {
    _private: (),
}

impl DevAuth {
    /// Construct devAuth, failing closed unless explicitly opted in and the
    /// environment is declared non-production (design §7.2). Emits a loud
    /// warning on success.
    ///
    /// **An undeclared environment counts as production.** devAuth exists so
    /// someone can trial Boswell locally with preset roles until they stand up a
    /// real identity provider — which means the realistic hazard is not malice
    /// but drift: trial it, like it, deploy it, and never think about identity
    /// again. Treating "no environment set" as safe would let exactly that path
    /// run fake identities in production forever. Saying `BOSWELL_ENV=development`
    /// out loud costs a trialist one variable and closes it.
    pub fn new(config: &DevAuthConfig) -> Result<Self, DevAuthError> {
        // Production lockout takes priority over opt-in.
        if config.production {
            return Err(DevAuthError::ProductionLockout);
        }
        if !config.environment_declared {
            return Err(DevAuthError::EnvironmentUndeclared);
        }
        if !config.allow_dev_auth {
            return Err(DevAuthError::NotOptedIn);
        }
        tracing::warn!("{}", DEV_AUTH_WARNING);
        Ok(Self { _private: () })
    }

    /// The startup banner operators should display.
    pub fn banner() -> &'static str {
        DEV_AUTH_BANNER
    }

    /// Build a tainted [`ProvenanceStamp`] for a write by `identity` (design §7.2).
    ///
    /// The stamp carries `dev_provider = true`, the identity's authority and
    /// nominal assurance, and the (possibly coerced) evidence. `timestamp` is
    /// Unix ms.
    pub fn stamp(
        &self,
        identity: DevIdentity,
        delegation_chain: DelegationChain,
        evidence: EvidenceType,
        timestamp: u64,
    ) -> ProvenanceStamp {
        tracing::warn!(
            "issuing devAuth stamp for '{}' — {}",
            identity.name(),
            DEV_AUTH_WARNING
        );
        ProvenanceStamp {
            author: identity.principal_id().to_string(),
            delegation_chain,
            authority: identity.authority(),
            evidence: identity.coerce_evidence(evidence),
            assurance: identity.nominal_assurance(),
            task_id: None,
            session_id: None,
            timestamp,
            dev_provider: true,
        }
    }
}

impl IdentityProvider for DevAuth {
    /// Always `true`: that is the whole point of this adapter (design §7.2).
    fn is_dev_provider(&self) -> bool {
        true
    }

    fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError> {
        match DevIdentity::parse(&credential.token) {
            Some(identity) => {
                tracing::warn!(
                    "devAuth assigned principal '{}' — {}",
                    identity.principal_id(),
                    DEV_AUTH_WARNING
                );
                Ok(identity.principal())
            }
            None => Err(AuthError::UnknownCredential),
        }
    }

    fn verify_delegation(&self, chain: &DelegationChain) -> DelegationVerdict {
        // Assurance from the recognized leaf identity; unknown chains are treated
        // as merely self-asserted.
        let assurance = chain
            .leaf()
            .and_then(DevIdentity::by_principal_id)
            .map(|i| i.nominal_assurance())
            .unwrap_or(Assurance::Asserted);
        DelegationVerdict {
            valid: true,
            assurance,
        }
    }
}

impl AuthorizationPolicy for DevAuth {
    fn authority_for(&self, principal: &Principal) -> Option<Authority> {
        DevIdentity::by_principal_id(&principal.id).map(|i| i.authority())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::entry_tier;

    fn dev_auth() -> DevAuth {
        DevAuth::new(&DevAuthConfig {
            allow_dev_auth: true,
            production: false,
            environment_declared: true,
        })
        .unwrap()
    }

    #[test]
    fn refuses_without_opt_in() {
        assert_eq!(
            DevAuth::new(&DevAuthConfig {
                allow_dev_auth: false,
                production: false,
                environment_declared: true,
            })
            .unwrap_err(),
            DevAuthError::NotOptedIn
        );
    }

    #[test]
    fn production_lockout_beats_opt_in() {
        assert_eq!(
            DevAuth::new(&DevAuthConfig {
                allow_dev_auth: true,
                production: true,
                environment_declared: true,
            })
            .unwrap_err(),
            DevAuthError::ProductionLockout
        );
    }

    /// An *undeclared* environment fails closed. The realistic hazard for a
    /// bring-up adapter is not malice but drift — trial it, deploy it, never
    /// think about identity again — and that path never sets `BOSWELL_ENV` at
    /// all. Reading "unset" as "not production" would keep fake identities alive
    /// through exactly that transition.
    #[test]
    fn an_undeclared_environment_is_treated_as_production() {
        assert_eq!(
            DevAuth::new(&DevAuthConfig {
                allow_dev_auth: true,
                production: false,
                environment_declared: false,
            })
            .unwrap_err(),
            DevAuthError::EnvironmentUndeclared
        );
    }

    /// The default config refuses, whatever the reason: nothing about an
    /// unconfigured devAuth may start.
    #[test]
    fn the_default_config_never_starts() {
        assert!(DevAuth::new(&DevAuthConfig::default()).is_err());
    }

    /// A blank `BOSWELL_ENV` is not a declaration.
    #[test]
    fn a_blank_environment_is_not_a_declaration() {
        let cfg = DevAuthConfig {
            allow_dev_auth: true,
            production: false,
            environment_declared: false,
        };
        assert!(matches!(
            DevAuth::new(&cfg),
            Err(DevAuthError::EnvironmentUndeclared)
        ));
    }

    /// The adapter reports itself as a development provider, which is what
    /// taints its stamps and marks the transport's responses without any
    /// production crate naming this crate (design §7.2).
    #[test]
    fn it_identifies_itself_as_a_development_provider() {
        assert!(dev_auth().is_dev_provider());
    }

    #[test]
    fn authenticate_maps_names_to_principals() {
        let auth = dev_auth();
        let p = auth
            .authenticate(&Credential::new("standard-worker"))
            .unwrap();
        assert_eq!(p.id, "agent:worker");
        assert_eq!(p.kind, PrincipalKind::Agent);
        assert!(auth.authenticate(&Credential::new("nobody")).is_err());
    }

    #[test]
    fn authority_matches_spec() {
        let worker = DevIdentity::StandardWorker.authority();
        assert_eq!(worker.max_tier, Tier::Task);
        assert!(worker.can(Op::Write) && !worker.can(Op::Endorse));

        let lead = DevIdentity::ProjectLeader.authority();
        assert_eq!(lead.max_tier, Tier::Project);
        assert!(lead.can(Op::Endorse));

        let mgr = DevIdentity::MemoryManager.authority();
        assert_eq!(mgr.max_tier, Tier::Permanent);
        assert!(mgr.can(Op::Curate));
        assert!(mgr.allows_namespace("anything:at:all"));

        let interloper = DevIdentity::UntrustedInterloper.authority();
        assert_eq!(interloper.max_tier, Tier::Ephemeral);
    }

    #[test]
    fn interloper_evidence_is_forced_weak() {
        assert_eq!(
            DevIdentity::UntrustedInterloper.coerce_evidence(EvidenceType::Observed),
            EvidenceType::ToolOutput
        );
        assert_eq!(
            DevIdentity::UntrustedInterloper.coerce_evidence(EvidenceType::Reported),
            EvidenceType::Reported
        );
        // Other identities are unaffected.
        assert_eq!(
            DevIdentity::ProjectLeader.coerce_evidence(EvidenceType::Observed),
            EvidenceType::Observed
        );
    }

    #[test]
    fn stamps_are_tainted() {
        let auth = dev_auth();
        let stamp = auth.stamp(
            DevIdentity::StandardWorker,
            DelegationChain(vec!["human:jd".into(), "agent:worker".into()]),
            EvidenceType::Observed,
            1,
        );
        assert!(stamp.dev_provider);
        assert_eq!(stamp.assurance, Assurance::Verified);
    }

    #[test]
    fn gradient_is_observable_through_entry_tier() {
        let auth = dev_auth();
        let chain = DelegationChain::default();

        // Interloper: forced weak evidence + ephemeral authority + asserted -> ephemeral.
        let s = auth.stamp(
            DevIdentity::UntrustedInterloper,
            chain.clone(),
            EvidenceType::Observed,
            1,
        );
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Ephemeral);

        // Worker: authority caps at task.
        let s = auth.stamp(
            DevIdentity::StandardWorker,
            chain.clone(),
            EvidenceType::Observed,
            1,
        );
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Task);

        // Project-leader: reaches project.
        let s = auth.stamp(
            DevIdentity::ProjectLeader,
            chain.clone(),
            EvidenceType::Observed,
            1,
        );
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Project);

        // Memory-manager: reaches permanent.
        let s = auth.stamp(DevIdentity::MemoryManager, chain, EvidenceType::Observed, 1);
        assert_eq!(entry_tier(Tier::Permanent, &s), Tier::Permanent);
    }

    #[test]
    fn verify_delegation_reads_leaf_assurance() {
        let auth = dev_auth();
        let chain = DelegationChain(vec!["project:lead".into()]);
        assert_eq!(
            auth.verify_delegation(&chain).assurance,
            Assurance::Attested
        );
        // Unknown leaf -> merely asserted.
        let unknown = DelegationChain(vec!["agent:whoever".into()]);
        assert_eq!(
            auth.verify_delegation(&unknown).assurance,
            Assurance::Asserted
        );
    }

    #[test]
    fn authorization_policy_resolves_known_principals() {
        let auth = dev_auth();
        let p = DevIdentity::ProjectLeader.principal();
        assert_eq!(auth.authority_for(&p).unwrap().max_tier, Tier::Project);
        assert!(auth
            .authority_for(&Principal::new("agent:unknown", PrincipalKind::Agent))
            .is_none());
    }
}
