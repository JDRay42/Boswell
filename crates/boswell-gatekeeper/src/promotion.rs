//! Tier-promotion authority for provenance-stamped entries (procedural memory,
//! Phase 3).
//!
//! This is the Gatekeeper pattern (agents advocate; the gatekeeper decides what
//! persists higher) pointed at the *agent hierarchy* rather than at claim
//! confidence (design §5). It is a **pure decision** over
//! [`CorroborationFacts`]: the store computes the facts, the Janitor sweep applies
//! the verdict. Nothing here touches storage.
//!
//! An entry climbs a tier when a higher-authority parent **endorses** it, OR
//! enough **distinct authors** corroborate it, OR it crosses an **effectiveness**
//! threshold — always bounded by the entry's climb ceiling
//! ([`CorroborationFacts::climb_ceiling`], itself a function of evidence type,
//! assurance, and authority). It falls a tier on a higher-authority
//! contradiction, repeated failure at the serving tier, or decay.

use boswell_domain::{CorroborationFacts, Tier};

/// Thresholds governing tier promotion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PromotionConfig {
    /// Distinct **delegation-chain roots** required to corroborate a climb
    /// (design §5.2, §8.1). Measured by provenance diversity, not raw author
    /// count, so correlated clones cannot manufacture corroboration.
    pub min_distinct_roots: usize,
    /// Effectiveness at or above which a procedure may climb (0.0..=1.0).
    pub effectiveness_threshold: f64,
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            min_distinct_roots: 2,
            effectiveness_threshold: 0.8,
        }
    }
}

/// The gatekeeper's verdict for one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionDecision {
    /// Climb to the given (higher) tier.
    Climb(Tier),
    /// No change.
    Hold,
    /// Fall to the given (lower) tier.
    Fall(Tier),
}

/// Decides tier climbs and falls for provenance-stamped entries.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromotionGatekeeper {
    config: PromotionConfig,
}

impl PromotionGatekeeper {
    /// Create a gatekeeper with the given configuration.
    pub fn new(config: PromotionConfig) -> Self {
        Self { config }
    }

    /// The configuration in force.
    pub fn config(&self) -> &PromotionConfig {
        &self.config
    }

    /// Decide whether an entry should climb, hold, or fall.
    ///
    /// Falls take priority over climbs: a contradicted, failing, or stale entry is
    /// never promoted in the same evaluation. An entry already at `Ephemeral` has
    /// nowhere to fall (deletion is the sweep's job), so it `Hold`s.
    pub fn evaluate(&self, facts: &CorroborationFacts) -> PromotionDecision {
        // --- Fall side (takes priority) ---
        if facts.contradicted_by_higher_authority || facts.failing || facts.stale {
            return match facts.current_tier.previous() {
                Some(prev) => PromotionDecision::Fall(prev),
                None => PromotionDecision::Hold,
            };
        }

        // --- Climb side ---
        let Some(next) = facts.current_tier.next() else {
            return PromotionDecision::Hold; // already Permanent
        };

        // Never climb above what evidence, assurance, and authority permit.
        if next.rank() > facts.climb_ceiling().rank() {
            return PromotionDecision::Hold;
        }

        let endorsed = facts
            .endorsed_max_tier
            .is_some_and(|t| t.rank() >= next.rank());
        // Corroboration is measured by provenance diversity, not raw author count,
        // so a swarm of clones sharing one delegation root cannot manufacture it
        // (design §8.1, the Sybil "pragmatic proxy").
        let corroborated = facts.distinct_delegation_roots >= self.config.min_distinct_roots;
        let effective = facts.effectiveness >= self.config.effectiveness_threshold;

        // Top-tier (permanent) promotion additionally requires a cross-authority
        // endorsement — a different org branch signing off (design §8.1). Below
        // top tier, any of endorsement / corroboration / effectiveness suffices.
        if next == Tier::Permanent {
            if endorsed && facts.cross_authority_endorsement {
                PromotionDecision::Climb(next)
            } else {
                PromotionDecision::Hold
            }
        } else if endorsed || corroborated || effective {
            PromotionDecision::Climb(next)
        } else {
            PromotionDecision::Hold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{Assurance, EvidenceType};

    /// A baseline set of facts: task tier, strong evidence/assurance, no climb
    /// trigger, no fall condition.
    fn facts() -> CorroborationFacts {
        CorroborationFacts {
            current_tier: Tier::Task,
            distinct_authors: 1,
            distinct_delegation_roots: 1,
            distinct_sessions: 1,
            distinct_evidence_types: 1,
            cross_authority_endorsement: false,
            endorsed_max_tier: None,
            author_max_tier: Tier::Permanent,
            best_evidence: EvidenceType::Observed,
            best_assurance: Assurance::Attested,
            effectiveness: 0.0,
            contradicted_by_higher_authority: false,
            failing: false,
            stale: false,
        }
    }

    #[test]
    fn holds_without_a_trigger() {
        let gk = PromotionGatekeeper::default();
        assert_eq!(gk.evaluate(&facts()), PromotionDecision::Hold);
    }

    #[test]
    fn climbs_on_distinct_root_corroboration() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.distinct_delegation_roots = 2;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Climb(Tier::Project));
    }

    #[test]
    fn clone_swarm_sharing_one_root_does_not_corroborate() {
        // Nine distinct author *identities* but a single delegation root — the
        // Sybil case (design §8, #1). Raw author count would climb; provenance
        // diversity does not.
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.distinct_authors = 9;
        f.distinct_delegation_roots = 1;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);
    }

    #[test]
    fn climbs_on_endorsement_by_higher_authority() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.endorsed_max_tier = Some(Tier::Project);
        assert_eq!(gk.evaluate(&f), PromotionDecision::Climb(Tier::Project));
    }

    #[test]
    fn climbs_on_effectiveness_threshold() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.effectiveness = 0.85;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Climb(Tier::Project));
    }

    #[test]
    fn climb_blocked_by_ceiling_even_when_corroborated() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.distinct_delegation_roots = 9;
        // tool_output caps the ceiling at task, so a task entry cannot climb.
        f.best_evidence = EvidenceType::ToolOutput;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);
    }

    #[test]
    fn low_assurance_cannot_reach_team_tier_without_endorsement() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.current_tier = Tier::Task;
        f.distinct_delegation_roots = 9;
        f.best_assurance = Assurance::Asserted; // ceiling task
        f.best_evidence = EvidenceType::Observed;
        f.author_max_tier = Tier::Permanent;
        // Asserted caps at task, so no climb to project.
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);

        // An attested project-tier endorsement raises the ceiling and lets it climb.
        f.endorsed_max_tier = Some(Tier::Project);
        f.best_assurance = Assurance::Attested;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Climb(Tier::Project));
    }

    #[test]
    fn falls_on_failure_contradiction_or_staleness() {
        let gk = PromotionGatekeeper::default();
        for setter in [
            |f: &mut CorroborationFacts| f.failing = true,
            |f: &mut CorroborationFacts| f.contradicted_by_higher_authority = true,
            |f: &mut CorroborationFacts| f.stale = true,
        ] {
            let mut f = facts();
            f.current_tier = Tier::Project;
            setter(&mut f);
            assert_eq!(gk.evaluate(&f), PromotionDecision::Fall(Tier::Task));
        }
    }

    #[test]
    fn fall_takes_priority_over_climb() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.current_tier = Tier::Project;
        f.distinct_delegation_roots = 9; // would climb
        f.failing = true; // but failing wins
        assert_eq!(gk.evaluate(&f), PromotionDecision::Fall(Tier::Task));
    }

    #[test]
    fn ephemeral_holds_instead_of_falling_below() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.current_tier = Tier::Ephemeral;
        f.failing = true;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);
    }

    #[test]
    fn permanent_holds_instead_of_climbing_above() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.current_tier = Tier::Permanent;
        f.distinct_delegation_roots = 9;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);
    }

    #[test]
    fn top_tier_requires_cross_authority_endorsement() {
        let gk = PromotionGatekeeper::default();
        let mut f = facts();
        f.current_tier = Tier::Project; // next is Permanent (top tier)
        f.author_max_tier = Tier::Permanent;
        f.endorsed_max_tier = Some(Tier::Permanent);
        f.distinct_delegation_roots = 9;
        f.effectiveness = 0.99;

        // Everything else says climb, but without a cross-authority endorsement a
        // top-tier promotion is held (design §8.1).
        f.cross_authority_endorsement = false;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Hold);

        // A different org branch signs off -> climb to permanent.
        f.cross_authority_endorsement = true;
        assert_eq!(gk.evaluate(&f), PromotionDecision::Climb(Tier::Permanent));
    }
}
