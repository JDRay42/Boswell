//! Cluster candidate claims into groups worth analysing.
//!
//! Two claims are grouped together when they are connected by either:
//!
//! 1. an explicit relationship edge between them (supports, references,
//!    contradicts, derived_from), or
//! 2. a shared subject (claims about the same entity).
//!
//! Grouping uses a union-find over the candidate set, so connectivity is
//! transitive: A–B and B–C place A, B, C in one cluster. This yields the
//! "high relationship density" clusters the architecture doc prioritises,
//! while still forming useful clusters when explicit edges are sparse.

use boswell_domain::{Claim, ClaimId, Relationship};
use std::collections::HashMap;

/// A simple union-find (disjoint-set) over claim indices.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path halving
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }
}

/// Group `claims` into clusters using relationship edges and shared subjects.
///
/// - `relationships` may reference claims outside the candidate set; only edges
///   whose *both* endpoints are candidates contribute to grouping.
/// - Clusters smaller than `min_size` are dropped.
/// - Clusters larger than `max_size` are truncated to `max_size` claims
///   (highest-confidence claims kept first) to keep prompts bounded.
///
/// The returned clusters are deterministic given the input ordering.
pub fn build_clusters(
    claims: Vec<Claim>,
    relationships: &[Relationship],
    min_size: usize,
    max_size: usize,
) -> Vec<Vec<Claim>> {
    if claims.is_empty() {
        return Vec::new();
    }

    // Map claim id -> index in the candidate vector.
    let index_of: HashMap<ClaimId, usize> = claims
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, i))
        .collect();

    let mut uf = UnionFind::new(claims.len());

    // 1) Union by explicit relationship edges within the candidate set.
    for rel in relationships {
        if let (Some(&a), Some(&b)) = (index_of.get(&rel.from_claim), index_of.get(&rel.to_claim)) {
            uf.union(a, b);
        }
    }

    // 2) Union by shared subject.
    let mut by_subject: HashMap<&str, usize> = HashMap::new();
    for (i, claim) in claims.iter().enumerate() {
        match by_subject.get(claim.subject.as_str()) {
            Some(&first) => uf.union(first, i),
            None => {
                by_subject.insert(claim.subject.as_str(), i);
            }
        }
    }

    // Collect components, preserving first-seen order for determinism.
    let mut order: Vec<usize> = Vec::new();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..claims.len() {
        let root = uf.find(i);
        groups.entry(root).or_insert_with(|| {
            order.push(root);
            Vec::new()
        });
        groups.get_mut(&root).unwrap().push(i);
    }

    // Materialise clusters (clone claims out of the candidate vector).
    let mut result = Vec::new();
    for root in order {
        let indices = &groups[&root];
        if indices.len() < min_size {
            continue;
        }

        let mut cluster: Vec<Claim> = indices.iter().map(|&i| claims[i].clone()).collect();

        if cluster.len() > max_size {
            // Keep the highest-confidence claims (by upper bound, then lower).
            cluster.sort_by(|a, b| {
                b.confidence
                    .1
                    .partial_cmp(&a.confidence.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        b.confidence
                            .0
                            .partial_cmp(&a.confidence.0)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
            });
            cluster.truncate(max_size);
        }

        result.push(cluster);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::relationship::RelationshipType;

    fn claim(subject: &str, object: &str) -> Claim {
        Claim {
            id: ClaimId::new(),
            namespace: "ns".to_string(),
            subject: subject.to_string(),
            predicate: "rel".to_string(),
            object: object.to_string(),
            source_type: "assertion".to_string(),
            confidence: (0.6, 0.8),
            tier: "task".to_string(),
            created_at: 0,
            stale_at: None,
        }
    }

    #[test]
    fn test_shared_subject_clusters() {
        let claims = vec![
            claim("entity:x", "a"),
            claim("entity:x", "b"),
            claim("entity:x", "c"),
            claim("entity:y", "d"),
        ];
        let clusters = build_clusters(claims, &[], 3, 12);
        // Only the three "entity:x" claims form a cluster of size >= 3.
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
    }

    #[test]
    fn test_relationship_bridges_subjects() {
        let a = claim("entity:a", "1");
        let b = claim("entity:b", "2");
        let c = claim("entity:c", "3");
        let rels = vec![
            Relationship::new(a.id, b.id, RelationshipType::Supports, 0.9, 0),
            Relationship::new(b.id, c.id, RelationshipType::References, 0.9, 0),
        ];
        let clusters = build_clusters(vec![a, b, c], &rels, 3, 12);
        // Transitive: a-b-c form one cluster despite distinct subjects.
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 3);
    }

    #[test]
    fn test_min_size_filter() {
        let claims = vec![claim("entity:x", "a"), claim("entity:x", "b")];
        let clusters = build_clusters(claims, &[], 3, 12);
        assert!(clusters.is_empty());
    }

    #[test]
    fn test_max_size_truncation() {
        let mut claims = Vec::new();
        for i in 0..10 {
            let mut c = claim("entity:x", &format!("obj{}", i));
            // Vary confidence so truncation keeps the strongest.
            c.confidence = (0.5, 0.5 + (i as f64) * 0.04);
            claims.push(c);
        }
        let clusters = build_clusters(claims, &[], 3, 5);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 5);
        // The kept claims should be the highest-confidence ones (upper >= 0.7).
        for c in &clusters[0] {
            assert!(c.confidence.1 >= 0.7 - f64::EPSILON, "kept {:?}", c.confidence);
        }
    }

    #[test]
    fn test_relationship_outside_candidate_set_ignored() {
        let a = claim("entity:a", "1");
        let b = claim("entity:b", "2");
        let outsider = ClaimId::new();
        let rels = vec![Relationship::new(a.id, outsider, RelationshipType::Supports, 0.9, 0)];
        // a and b share no subject and only a references a non-candidate.
        let clusters = build_clusters(vec![a, b], &rels, 2, 12);
        // Neither reaches size 2 as a connected group.
        assert!(clusters.is_empty());
    }
}
