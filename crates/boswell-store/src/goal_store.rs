//! SQLite persistence and single-hop traversal for [`Goal`] and [`GoalEdge`]
//! (procedural memory, Phase 2).
//!
//! Goals are stored skinny in the `goals` table; their decomposition lives in
//! `goal_edges`, keyed by parent, so a hop is one indexed adjacency read (§4.2).
//! [`SqliteStore::expand`] realizes the deterministic `expand(node, context)`
//! surface of `docs/architecture/15-procedural-memory.md` §4.1: read the parent's
//! edges, drop those whose edge-local preconditions do not hold against the claim
//! store, rank the survivors by effectiveness then context-tag match, and return
//! them alongside any `decide`-role decision aids and the raw factor readings
//! consulted. The store surfaces; the agent decides.
//!
//! Goals form a DAG; [`SqliteStore::add_goal_edge`] rejects any sub-goal edge
//! that would close a cycle (§8, open problem #6).

use crate::procedure_store::{from_json, like_escape, to_json, PreconditionDto};
use crate::{SqliteStore, StoreError};
use boswell_domain::traits::{ClaimQuery, ClaimStore, GoalStore};
use boswell_domain::{
    ChildKind, ChildRef, Claim, CollectOutcome, EdgeRole, ExpandResult, ExpandedCandidate,
    FactorReading, Goal, GoalEdge, GoalId, GoalQuery, GraphIntegrity, Precondition, ProcedureId,
    Tier, TraversalContext,
};
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

/// The full column list for `goals`, read by name in [`SqliteStore::row_to_goal`].
const GOAL_COLUMNS: &str =
    "id, namespace, name, intent, definition_of_done, tier, created_at, updated_at, stale_at";

impl SqliteStore {
    fn goal_id_to_bytes(id: GoalId) -> Vec<u8> {
        id.value().to_be_bytes().to_vec()
    }

    fn bytes_to_u128(bytes: &[u8]) -> Result<u128, StoreError> {
        if bytes.len() != 16 {
            return Err(StoreError::InvalidData(format!(
                "Expected 16 bytes for a goal/procedure id, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        Ok(u128::from_be_bytes(arr))
    }

    /// Insert a goal, or replace it in place if one with the same id exists.
    ///
    /// This persists only the skinny node; its decomposition is managed
    /// separately via [`SqliteStore::add_goal_edge`]. Returns the goal's id.
    pub fn upsert_goal(&mut self, goal: &Goal) -> Result<GoalId, StoreError> {
        let id_bytes = Self::goal_id_to_bytes(goal.id);
        let dod = to_json(&goal.definition_of_done)?;

        self.conn.execute(
            "INSERT INTO goals (
                id, namespace, name, intent, definition_of_done, tier,
                created_at, updated_at, stale_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                namespace = excluded.namespace, name = excluded.name,
                intent = excluded.intent,
                definition_of_done = excluded.definition_of_done,
                tier = excluded.tier, created_at = excluded.created_at,
                updated_at = excluded.updated_at, stale_at = excluded.stale_at",
            params![
                &id_bytes,
                &goal.namespace,
                &goal.name,
                &goal.intent,
                &dod,
                goal.tier.as_str(),
                goal.created_at as i64,
                goal.updated_at as i64,
                goal.stale_at.map(|t| t as i64),
            ],
        )?;
        Ok(goal.id)
    }

    /// Fetch a goal by id, or `None` if it does not exist.
    pub fn get_goal(&self, id: GoalId) -> Result<Option<Goal>, StoreError> {
        let id_bytes = Self::goal_id_to_bytes(id);
        self.conn
            .query_row(
                &format!("SELECT {} FROM goals WHERE id = ?1", GOAL_COLUMNS),
                params![&id_bytes],
                Self::row_to_goal,
            )
            .optional()?
            .transpose()
    }

    /// Query goals by namespace prefix and/or an intent substring — the entry
    /// point for a descent ("I need to eat" -> root goals).
    ///
    /// Results are ordered by id (chronological). Semantic intent match via the
    /// embedding path (ADR-005) is a TODO; Phase 2 uses a case-insensitive
    /// substring match, mirroring procedure retrieval.
    pub fn query_goals(
        &self,
        namespace: Option<&str>,
        intent_contains: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Goal>, StoreError> {
        let mut sql = format!("SELECT {} FROM goals WHERE 1=1", GOAL_COLUMNS);
        let mut sql_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(ns) = namespace {
            sql.push_str(" AND namespace LIKE ?");
            sql_params.push(Box::new(format!("{}%", ns)));
        }
        if let Some(intent) = intent_contains {
            // TODO(procedural-memory): semantic intent match via HNSW (§4.2).
            sql.push_str(" AND intent LIKE ? ESCAPE '\\'");
            sql_params.push(Box::new(format!("%{}%", like_escape(intent))));
        }
        sql.push_str(" ORDER BY id");
        if let Some(limit) = limit {
            sql.push_str(" LIMIT ?");
            sql_params.push(Box::new(limit));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = sql_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(&param_refs[..], Self::row_to_goal)?
            .collect::<Result<Vec<Result<Goal, StoreError>>, rusqlite::Error>>()?;
        rows.into_iter().collect()
    }

    /// Add (or update in place) a decomposition edge under a parent goal.
    ///
    /// Edges are unique per `(parent, child, role)`; re-adding the same triple
    /// updates its preconditions/hints/effectiveness. A sub-goal edge that would
    /// close a cycle in the goal DAG is rejected with [`StoreError::Cycle`]
    /// (§8, #6); procedure edges are leaves and never form cycles.
    ///
    /// `now` (Unix ms) stamps the edge's create/update time.
    pub fn add_goal_edge(&mut self, edge: &GoalEdge, now: u64) -> Result<(), StoreError> {
        if let ChildRef::Goal(child) = edge.child {
            if self.would_create_cycle(edge.parent, child)? {
                return Err(StoreError::Cycle(format!("{} -> {}", edge.parent, child)));
            }
        }

        let parent_bytes = Self::goal_id_to_bytes(edge.parent);
        let child_bytes = edge.child.id_value().to_be_bytes().to_vec();
        let preconditions = to_json(
            &edge
                .preconditions
                .iter()
                .map(PreconditionDto::from)
                .collect::<Vec<_>>(),
        )?;
        let context_tags = to_json(&edge.context_tags)?;

        self.conn.execute(
            "INSERT INTO goal_edges (
                parent_goal_id, child_kind, child_id, role,
                preconditions, context_tags, usage_notes, cached_effectiveness,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(parent_goal_id, child_kind, child_id, role) DO UPDATE SET
                preconditions = excluded.preconditions,
                context_tags = excluded.context_tags,
                usage_notes = excluded.usage_notes,
                cached_effectiveness = excluded.cached_effectiveness,
                updated_at = excluded.updated_at",
            params![
                &parent_bytes,
                edge.child.kind().as_str(),
                &child_bytes,
                edge.role.as_str(),
                &preconditions,
                &context_tags,
                &edge.usage_notes,
                edge.cached_effectiveness,
                now as i64,
            ],
        )?;
        Ok(())
    }

    /// Read a parent goal's edges directly (a single adjacency read), ordered by
    /// insertion. This is the raw form; [`SqliteStore::expand`] adds precondition
    /// filtering and ranking.
    pub fn get_goal_edges(&self, parent: GoalId) -> Result<Vec<GoalEdge>, StoreError> {
        let parent_bytes = Self::goal_id_to_bytes(parent);
        let mut stmt = self.conn.prepare(
            "SELECT child_kind, child_id, role, preconditions, context_tags, usage_notes, \
             cached_effectiveness FROM goal_edges WHERE parent_goal_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![&parent_bytes], move |row| {
                Self::row_to_goal_edge(parent, row)
            })?
            .collect::<Result<Vec<Result<GoalEdge, StoreError>>, rusqlite::Error>>()?;
        rows.into_iter().collect()
    }

    /// Expand one hop: surface the parent's ranked, precondition-passing children
    /// plus decision aids and factor readings (design §4.1).
    ///
    /// Deterministic order: effectiveness descending, then context-tag match
    /// descending, then child id ascending. An unknown or childless goal yields
    /// an empty surface (callers can [`get_goal`](SqliteStore::get_goal) to tell
    /// the two apart). `now` is currently unused for ranking (edges carry cached
    /// effectiveness) but is part of the stable signature for future recency use.
    pub fn expand(
        &self,
        goal_id: GoalId,
        context: &TraversalContext,
        _now: u64,
    ) -> Result<ExpandResult, StoreError> {
        let edges = self.get_goal_edges(goal_id)?;
        if edges.is_empty() {
            return Ok(ExpandResult::default());
        }

        // Fetch the context slice once for the whole hop (§4.2): pull claims at or
        // above the lowest precondition floor, then evaluate every check in-process.
        let has_preconditions = edges.iter().any(|e| !e.preconditions.is_empty());
        let slice = if has_preconditions {
            let min_conf = edges
                .iter()
                .flat_map(|e| &e.preconditions)
                .map(|pc| pc.check.min_confidence)
                .fold(f64::INFINITY, f64::min);
            let cq = ClaimQuery {
                min_confidence: Some(min_conf),
                ..ClaimQuery::default()
            };
            self.query_claims(&cq)?
        } else {
            Vec::new()
        };

        let mut candidates = Vec::new();
        let mut decision_aids = Vec::new();
        let mut factor_readings = Vec::new();
        let mut seen: HashSet<(String, String, String)> = HashSet::new();

        for edge in edges {
            if !preconditions_hold(&edge.preconditions, &slice) {
                continue;
            }
            collect_readings(&edge.preconditions, &slice, &mut seen, &mut factor_readings);

            let context_match = edge
                .context_tags
                .iter()
                .filter(|t| context.context_tags.contains(t))
                .count();
            let candidate = ExpandedCandidate {
                child: edge.child,
                role: edge.role,
                context_tags: edge.context_tags,
                usage_notes: edge.usage_notes,
                effectiveness: edge.cached_effectiveness,
                context_match,
            };
            match edge.role {
                EdgeRole::Accomplish => candidates.push(candidate),
                EdgeRole::Decide => decision_aids.push(candidate),
            }
        }

        rank_candidates(&mut candidates);
        rank_candidates(&mut decision_aids);

        Ok(ExpandResult {
            candidates,
            decision_aids,
            factor_readings,
        })
    }

    /// Recompute the cached effectiveness of a parent's *procedure* edges from the
    /// live procedure counters (the maintenance seam a Janitor sweep drives).
    ///
    /// Goal-child edges are left unchanged (aggregating a sub-goal's effectiveness
    /// is out of scope for Phase 2). Returns the number of edges updated. `now`
    /// (Unix ms) is the reference time for [`boswell_domain::Procedure::effectiveness`].
    pub fn recompute_goal_edge_effectiveness(
        &mut self,
        parent: GoalId,
        now: u64,
    ) -> Result<usize, StoreError> {
        let mut updated = 0;
        for edge in self.get_goal_edges(parent)? {
            if let ChildRef::Procedure(pid) = edge.child {
                if let Some(procedure) = self.get_procedure(pid)? {
                    let eff = procedure.effectiveness(now);
                    let parent_bytes = Self::goal_id_to_bytes(parent);
                    let child_bytes = pid.value().to_be_bytes().to_vec();
                    self.conn.execute(
                        "UPDATE goal_edges SET cached_effectiveness = ?1, updated_at = ?2 \
                         WHERE parent_goal_id = ?3 AND child_kind = 'procedure' \
                         AND child_id = ?4 AND role = ?5",
                        params![
                            eff,
                            now as i64,
                            &parent_bytes,
                            &child_bytes,
                            edge.role.as_str()
                        ],
                    )?;
                    updated += 1;
                }
            }
        }
        Ok(updated)
    }

    /// Whether adding a `parent -> child` sub-goal edge would create a cycle, i.e.
    /// `child == parent` or `parent` is already reachable from `child` over
    /// goal-to-goal edges.
    fn would_create_cycle(&self, parent: GoalId, child: GoalId) -> Result<bool, StoreError> {
        if parent == child {
            return Ok(true);
        }
        let mut stack = vec![child];
        let mut visited: HashSet<u128> = HashSet::new();
        while let Some(node) = stack.pop() {
            if !visited.insert(node.value()) {
                continue;
            }
            for next in self.child_goal_ids(node)? {
                if next == parent {
                    return Ok(true);
                }
                stack.push(next);
            }
        }
        Ok(false)
    }

    /// The sub-goal children of a goal (goal-kind edges only).
    fn child_goal_ids(&self, parent: GoalId) -> Result<Vec<GoalId>, StoreError> {
        let parent_bytes = Self::goal_id_to_bytes(parent);
        let mut stmt = self.conn.prepare(
            "SELECT child_id FROM goal_edges WHERE parent_goal_id = ?1 AND child_kind = 'goal'",
        )?;
        let ids = stmt
            .query_map(params![&parent_bytes], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                Ok(bytes)
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        ids.into_iter()
            .map(|b| Self::bytes_to_u128(&b).map(GoalId::from_value))
            .collect()
    }

    fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Goal, StoreError>> {
        let decode = |row: &rusqlite::Row<'_>| -> Result<Goal, StoreError> {
            let id_bytes: Vec<u8> = row.get("id")?;
            let tier_str: String = row.get("tier")?;
            let tier = Tier::parse(&tier_str)
                .ok_or_else(|| StoreError::InvalidData(format!("Unknown tier: {}", tier_str)))?;
            let definition_of_done: Vec<String> =
                from_json(&row.get::<_, String>("definition_of_done")?)?;
            let stale_at: Option<i64> = row.get("stale_at")?;
            Ok(Goal {
                id: GoalId::from_value(Self::bytes_to_u128(&id_bytes)?),
                namespace: row.get("namespace")?,
                name: row.get("name")?,
                intent: row.get("intent")?,
                definition_of_done,
                tier,
                created_at: row.get::<_, i64>("created_at")? as u64,
                updated_at: row.get::<_, i64>("updated_at")? as u64,
                stale_at: stale_at.map(|t| t as u64),
            })
        };
        Ok(decode(row))
    }

    fn row_to_goal_edge(
        parent: GoalId,
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<Result<GoalEdge, StoreError>> {
        let decode = |row: &rusqlite::Row<'_>| -> Result<GoalEdge, StoreError> {
            let child_kind_str: String = row.get("child_kind")?;
            let child_kind = ChildKind::parse(&child_kind_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown child kind: {}", child_kind_str))
            })?;
            let child_bytes: Vec<u8> = row.get("child_id")?;
            let child_value = Self::bytes_to_u128(&child_bytes)?;
            let child = match child_kind {
                ChildKind::Goal => ChildRef::Goal(GoalId::from_value(child_value)),
                ChildKind::Procedure => ChildRef::Procedure(ProcedureId::from_value(child_value)),
            };
            let role_str: String = row.get("role")?;
            let role = EdgeRole::parse(&role_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown edge role: {}", role_str))
            })?;
            let preconditions_dto: Vec<PreconditionDto> =
                from_json(&row.get::<_, String>("preconditions")?)?;
            let preconditions = preconditions_dto
                .into_iter()
                .map(PreconditionDto::into_domain)
                .collect::<Result<Vec<_>, _>>()?;
            let context_tags: Vec<String> = from_json(&row.get::<_, String>("context_tags")?)?;
            Ok(GoalEdge {
                parent,
                child,
                role,
                preconditions,
                context_tags,
                usage_notes: row.get("usage_notes")?,
                cached_effectiveness: row.get("cached_effectiveness")?,
            })
        };
        Ok(decode(row))
    }
}

/// Whether every precondition holds against an already-fetched claim slice.
fn preconditions_hold(preconditions: &[Precondition], slice: &[Claim]) -> bool {
    preconditions.iter().all(|pc| {
        let m = &pc.check.match_pattern;
        let matched = slice.iter().any(|c| {
            c.subject == m.subject
                && c.predicate == m.predicate
                && c.object == m.object
                && c.confidence.0 >= pc.check.min_confidence
        });
        match pc.check.expect {
            boswell_domain::Expect::Exists => matched,
            boswell_domain::Expect::Absent => !matched,
        }
    })
}

/// Append the distinct claims (by triple) that a set of preconditions references
/// and that are present in the slice — the "why" behind a surfaced edge.
fn collect_readings(
    preconditions: &[Precondition],
    slice: &[Claim],
    seen: &mut HashSet<(String, String, String)>,
    out: &mut Vec<FactorReading>,
) {
    for pc in preconditions {
        let m = &pc.check.match_pattern;
        for c in slice {
            if c.subject == m.subject && c.predicate == m.predicate && c.object == m.object {
                let key = (c.subject.clone(), c.predicate.clone(), c.object.clone());
                if seen.insert(key) {
                    out.push(FactorReading {
                        subject: c.subject.clone(),
                        predicate: c.predicate.clone(),
                        object: c.object.clone(),
                        confidence: c.confidence,
                    });
                }
            }
        }
    }
}

/// Rank candidates deterministically: effectiveness desc, context match desc,
/// then child id ascending as a stable final tie-break.
fn rank_candidates(candidates: &mut [ExpandedCandidate]) {
    candidates.sort_by(|a, b| {
        b.effectiveness
            .partial_cmp(&a.effectiveness)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.context_match.cmp(&a.context_match))
            .then_with(|| a.child.id_value().cmp(&b.child.id_value()))
    });
}

/// Goal-traversal transport surface (design §3.2, §4.1).
///
/// Bridges the inherent `SqliteStore` goal methods to the domain [`GoalStore`]
/// trait so the gRPC service can serve traversal over a generic store handle.
impl GoalStore for SqliteStore {
    fn supports_goals(&self) -> bool {
        true
    }

    fn get_goal(&self, id: GoalId) -> Result<Option<Goal>, Self::Error> {
        SqliteStore::get_goal(self, id)
    }

    fn query_goals(&self, query: &GoalQuery) -> Result<Vec<Goal>, Self::Error> {
        SqliteStore::query_goals(
            self,
            query.namespace.as_deref(),
            query.intent_contains.as_deref(),
            query.limit,
        )
    }

    fn expand(
        &self,
        goal_id: GoalId,
        context: &TraversalContext,
        now: u64,
    ) -> Result<ExpandResult, Self::Error> {
        SqliteStore::expand(self, goal_id, context, now)
    }
}

// ---- Graph integrity under decay (design §8, open problem #5) ----

impl SqliteStore {
    /// Edges pointing **at** `child` — the reverse adjacency traversal never
    /// needs but collection does.
    fn edges_referencing(&self, child: ChildRef) -> Result<Vec<GoalId>, StoreError> {
        let child_bytes = child.id_value().to_be_bytes().to_vec();
        let mut stmt = self.conn.prepare(
            "SELECT parent_goal_id FROM goal_edges WHERE child_kind = ?1 AND child_id = ?2",
        )?;
        let rows = stmt.query_map(params![child.kind().as_str(), &child_bytes], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        let mut parents = Vec::new();
        for row in rows {
            parents.push(GoalId::from_value(Self::bytes_to_u128(&row?)?));
        }
        Ok(parents)
    }

    /// Remove every edge pointing at `child`. Returns how many went.
    fn remove_edges_referencing(&mut self, child: ChildRef) -> Result<usize, StoreError> {
        let child_bytes = child.id_value().to_be_bytes().to_vec();
        Ok(self.conn.execute(
            "DELETE FROM goal_edges WHERE child_kind = ?1 AND child_id = ?2",
            params![child.kind().as_str(), &child_bytes],
        )?)
    }

    /// Collect a goal, keeping the navigable graph whole per `policy` (§8 #5).
    ///
    /// Nothing here decides *whether* a goal is stale — that is the Janitor's
    /// job. This decides only what happens to the graph around it once that
    /// call has been made.
    ///
    /// Returns a [`CollectOutcome`] describing what actually happened, including
    /// the case where the policy refused to collect at all.
    pub fn collect_goal(
        &mut self,
        id: GoalId,
        policy: GraphIntegrity,
        now: u64,
    ) -> Result<CollectOutcome, StoreError> {
        if self.get_goal(id)?.is_none() {
            return Ok(CollectOutcome::default());
        }

        let referencing = self.edges_referencing(ChildRef::Goal(id))?;

        // Pinning is checked before anything is touched: a refusal must leave the
        // graph exactly as it was.
        if policy == GraphIntegrity::PinChildren && !referencing.is_empty() {
            return Ok(CollectOutcome {
                collected: false,
                pinned_by: referencing.len(),
                ..CollectOutcome::default()
            });
        }

        // The goal's own children, captured before its outgoing edges are cut.
        let children = self.get_goal_edges(id)?;

        let mut reparented = 0;
        if policy == GraphIntegrity::CascadeAndReparent {
            // "Nearest live ancestor" is any live parent of the node being
            // collected. With several, the lowest id is chosen so the result is
            // deterministic rather than dependent on row order.
            let mut ancestors: Vec<GoalId> = referencing.clone();
            ancestors.sort_by_key(|g| g.value());
            if let Some(&ancestor) = ancestors.first() {
                for edge in &children {
                    // Skip a re-parent that would point the ancestor at itself.
                    if edge.child == ChildRef::Goal(ancestor) {
                        continue;
                    }
                    let moved = GoalEdge {
                        parent: ancestor,
                        ..edge.clone()
                    };
                    // A re-parent that would close a cycle is dropped rather
                    // than forced: an unreachable child is recoverable, a cyclic
                    // graph is not.
                    match self.add_goal_edge(&moved, now) {
                        Ok(()) => reparented += 1,
                        Err(StoreError::Cycle(_)) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }

        let inbound = self.remove_edges_referencing(ChildRef::Goal(id))?;

        // Outgoing edges go with the row via ON DELETE CASCADE, so they are
        // counted here rather than deleted separately.
        let outbound = children.len();
        self.conn.execute(
            "DELETE FROM goals WHERE id = ?1",
            params![Self::goal_id_to_bytes(id)],
        )?;

        // A child is orphaned if nothing points at it any more.
        let mut orphaned = 0;
        for edge in &children {
            if self.edges_referencing(edge.child)?.is_empty() {
                orphaned += 1;
            }
        }

        Ok(CollectOutcome {
            collected: true,
            pinned_by: referencing.len(),
            edges_removed: inbound + outbound,
            children_reparented: reparented,
            children_orphaned: orphaned,
        })
    }

    /// Collect a procedure, keeping the navigable graph whole per `policy`.
    ///
    /// A procedure is always a leaf, so it has no children to re-parent and
    /// `CascadeAndReparent` and `CascadeAndOrphan` behave identically here. Its
    /// outstanding receipts are deliberately left alone: a receipt is an
    /// obligation someone already took on, and the report path already handles a
    /// procedure that no longer exists.
    pub fn collect_procedure(
        &mut self,
        id: ProcedureId,
        policy: GraphIntegrity,
    ) -> Result<CollectOutcome, StoreError> {
        if self.get_procedure(id)?.is_none() {
            return Ok(CollectOutcome::default());
        }

        let referencing = self.edges_referencing(ChildRef::Procedure(id))?;
        if policy == GraphIntegrity::PinChildren && !referencing.is_empty() {
            return Ok(CollectOutcome {
                collected: false,
                pinned_by: referencing.len(),
                ..CollectOutcome::default()
            });
        }

        let removed = self.remove_edges_referencing(ChildRef::Procedure(id))?;
        self.conn.execute(
            "DELETE FROM procedures WHERE id = ?1",
            params![id.value().to_be_bytes().to_vec()],
        )?;

        Ok(CollectOutcome {
            collected: true,
            pinned_by: referencing.len(),
            edges_removed: removed,
            children_reparented: 0,
            children_orphaned: 0,
        })
    }

    /// Every edge whose child row no longer exists.
    ///
    /// A dangling edge is always a bug — `expand` would surface a candidate that
    /// cannot be fetched, and it would look perfectly healthy because the edge
    /// carries its own cached effectiveness and usage notes (§4.2). This is the
    /// detector; [`prune_dangling_goal_edges`](SqliteStore::prune_dangling_goal_edges)
    /// is the repair.
    pub fn dangling_goal_edges(&self) -> Result<Vec<GoalEdge>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT parent_goal_id FROM goal_edges GROUP BY parent_goal_id")?;
        let parents = stmt
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut dangling = Vec::new();
        for parent in parents {
            let parent = GoalId::from_value(Self::bytes_to_u128(&parent)?);
            for edge in self.get_goal_edges(parent)? {
                let alive = match edge.child {
                    ChildRef::Goal(id) => self.get_goal(id)?.is_some(),
                    ChildRef::Procedure(id) => self.get_procedure(id)?.is_some(),
                };
                if !alive {
                    dangling.push(edge);
                }
            }
        }
        Ok(dangling)
    }

    /// Delete every edge whose child no longer exists, returning how many went.
    ///
    /// Collection through [`collect_goal`](SqliteStore::collect_goal) /
    /// [`collect_procedure`](SqliteStore::collect_procedure) never creates a
    /// dangling edge, so this is a repair pass for graphs damaged another way —
    /// a direct `DELETE`, a restored backup, an older Boswell.
    pub fn prune_dangling_goal_edges(&mut self) -> Result<usize, StoreError> {
        let dangling = self.dangling_goal_edges()?;
        let mut pruned = 0;
        for edge in dangling {
            let child_bytes = edge.child.id_value().to_be_bytes().to_vec();
            pruned += self.conn.execute(
                "DELETE FROM goal_edges \
                 WHERE parent_goal_id = ?1 AND child_kind = ?2 AND child_id = ?3 AND role = ?4",
                params![
                    Self::goal_id_to_bytes(edge.parent),
                    edge.child.kind().as_str(),
                    &child_bytes,
                    edge.role.as_str(),
                ],
            )?;
        }
        Ok(pruned)
    }

    /// Goals no live edge points at — reachable only by query or direct id.
    ///
    /// Not an error condition: a root goal is orphaned by definition, and so is
    /// a child left standing by [`GraphIntegrity::CascadeAndOrphan`]. This is
    /// the measurement a Janitor sweep would use to decide what has fallen out
    /// of the navigable graph.
    pub fn orphaned_goals(&self) -> Result<Vec<Goal>, StoreError> {
        let mut orphans = Vec::new();
        for goal in self.query_goals(None, None, None)? {
            if self.edges_referencing(ChildRef::Goal(goal.id))?.is_empty() {
                orphans.push(goal);
            }
        }
        Ok(orphans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{
        ClaimId, ClaimMatch, Expect, Precondition, PreconditionCheck, Procedure, ProcedureSource,
    };

    const NOW: u64 = 1_700_000_000_000;

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    fn goal(name: &str) -> Goal {
        Goal {
            id: GoalId::new(),
            namespace: "person:jd".into(),
            name: name.into(),
            intent: format!("intent: {}", name),
            definition_of_done: vec![format!("{} is done", name)],
            tier: Tier::Project,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        }
    }

    fn edge(parent: GoalId, child: ChildRef, role: EdgeRole, eff: f64) -> GoalEdge {
        GoalEdge {
            parent,
            child,
            role,
            preconditions: vec![],
            context_tags: vec![],
            usage_notes: String::new(),
            cached_effectiveness: eff,
        }
    }

    fn precondition(subject: &str, predicate: &str, object: &str, expect: Expect) -> Precondition {
        Precondition {
            kind: "resource".into(),
            description: format!("{} {} {}", subject, predicate, object),
            check: PreconditionCheck {
                match_pattern: ClaimMatch {
                    subject: subject.into(),
                    predicate: predicate.into(),
                    object: object.into(),
                },
                min_confidence: 0.6,
                expect,
            },
        }
    }

    fn assert_pantry_claim(store: &mut SqliteStore, object: &str) {
        let claim = Claim::new(
            ClaimId::new(),
            "person:jd".into(),
            "person:jd".into(),
            "attr:in-pantry".into(),
            object.into(),
            (0.7, 0.8),
            "project".into(),
            NOW,
        );
        store.assert_claim(claim).unwrap();
    }

    fn procedure(id: ProcedureId, name: &str) -> Procedure {
        Procedure {
            id,
            namespace: "person:jd".into(),
            name: name.into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "goal:person:jd/cook-eggs".into(),
            intent: name.into(),
            tags: vec![],
            parameters: vec![],
            preconditions: vec![],
            required_tools: vec![],
            postconditions: vec![],
            est_duration_sec: None,
            usage_notes: String::new(),
            context_tags: vec![],
            body_format: boswell_domain::BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "body".into(),
            tier: Tier::Project,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            unknown_count: 0,
            last_used_at: None,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        }
    }

    #[test]
    fn goal_upsert_get_roundtrip() {
        let mut store = store();
        let g = goal("prepare-breakfast");
        let id = store.upsert_goal(&g).unwrap();
        assert_eq!(store.get_goal(id).unwrap().unwrap(), g);
        assert!(store.get_goal(GoalId::new()).unwrap().is_none());
    }

    #[test]
    fn goal_upsert_replaces_in_place() {
        let mut store = store();
        let mut g = goal("g");
        store.upsert_goal(&g).unwrap();
        g.intent = "revised".into();
        g.definition_of_done = vec!["new dod".into()];
        store.upsert_goal(&g).unwrap();
        let got = store.get_goal(g.id).unwrap().unwrap();
        assert_eq!(got.intent, "revised");
        assert_eq!(got.definition_of_done, vec!["new dod".to_string()]);
    }

    #[test]
    fn expand_ranks_accomplish_and_separates_decision_aids() {
        let mut store = store();
        let parent = goal("prepare-breakfast");
        store.upsert_goal(&parent).unwrap();

        let strong = ChildRef::Procedure(ProcedureId::new());
        let weak = ChildRef::Procedure(ProcedureId::new());
        let aid = ChildRef::Procedure(ProcedureId::new());

        // Insert weak first to prove ranking is by effectiveness, not insert order.
        store
            .add_goal_edge(&edge(parent.id, weak, EdgeRole::Accomplish, 0.4), NOW)
            .unwrap();
        store
            .add_goal_edge(&edge(parent.id, strong, EdgeRole::Accomplish, 0.9), NOW)
            .unwrap();
        store
            .add_goal_edge(&edge(parent.id, aid, EdgeRole::Decide, 0.5), NOW)
            .unwrap();

        let result = store
            .expand(parent.id, &TraversalContext::default(), NOW)
            .unwrap();
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[0].child, strong);
        assert_eq!(result.candidates[1].child, weak);
        assert_eq!(result.decision_aids.len(), 1);
        assert_eq!(result.decision_aids[0].child, aid);
    }

    #[test]
    fn expand_filters_on_edge_preconditions_and_returns_readings() {
        let mut store = store();
        assert_pantry_claim(&mut store, "ingredient:eggs");

        let parent = goal("cook-eggs");
        store.upsert_goal(&parent).unwrap();

        let eggs_child = ChildRef::Procedure(ProcedureId::new());
        let mut eggs_edge = edge(parent.id, eggs_child, EdgeRole::Accomplish, 0.8);
        eggs_edge.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:eggs",
            Expect::Exists,
        )];

        let flour_child = ChildRef::Procedure(ProcedureId::new());
        let mut flour_edge = edge(parent.id, flour_child, EdgeRole::Accomplish, 0.9);
        flour_edge.preconditions = vec![precondition(
            "person:jd",
            "attr:in-pantry",
            "ingredient:flour",
            Expect::Exists,
        )];

        store.add_goal_edge(&eggs_edge, NOW).unwrap();
        store.add_goal_edge(&flour_edge, NOW).unwrap();

        let result = store
            .expand(parent.id, &TraversalContext::default(), NOW)
            .unwrap();
        // Flour is absent, so despite higher effectiveness that edge is dropped.
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.candidates[0].child, eggs_child);
        // The eggs claim is surfaced as the factor reading behind the survivor.
        assert_eq!(result.factor_readings.len(), 1);
        assert_eq!(result.factor_readings[0].object, "ingredient:eggs");
    }

    #[test]
    fn expand_context_match_breaks_effectiveness_ties() {
        let mut store = store();
        let parent = goal("cook-eggs");
        store.upsert_goal(&parent).unwrap();

        let quick = ChildRef::Procedure(ProcedureId::from_value(1));
        let fancy = ChildRef::Procedure(ProcedureId::from_value(2));
        let mut quick_edge = edge(parent.id, quick, EdgeRole::Accomplish, 0.7);
        quick_edge.context_tags = vec!["time:quick".into()];
        let mut fancy_edge = edge(parent.id, fancy, EdgeRole::Accomplish, 0.7);
        fancy_edge.context_tags = vec!["mood:fancy".into()];
        store.add_goal_edge(&quick_edge, NOW).unwrap();
        store.add_goal_edge(&fancy_edge, NOW).unwrap();

        let ctx = TraversalContext {
            context_tags: vec!["time:quick".into()],
        };
        let result = store.expand(parent.id, &ctx, NOW).unwrap();
        assert_eq!(result.candidates[0].child, quick);
        assert_eq!(result.candidates[0].context_match, 1);
        assert_eq!(result.candidates[1].context_match, 0);
    }

    #[test]
    fn add_goal_edge_rejects_cycles() {
        let mut store = store();
        let a = goal("a");
        let b = goal("b");
        let c = goal("c");
        for g in [&a, &b, &c] {
            store.upsert_goal(g).unwrap();
        }
        // a -> b -> c
        store
            .add_goal_edge(
                &edge(a.id, ChildRef::Goal(b.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap();
        store
            .add_goal_edge(
                &edge(b.id, ChildRef::Goal(c.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap();

        // c -> a would close the cycle.
        let err = store
            .add_goal_edge(
                &edge(c.id, ChildRef::Goal(a.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::Cycle(_)));

        // Self-loop is also rejected.
        let err = store
            .add_goal_edge(
                &edge(a.id, ChildRef::Goal(a.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap_err();
        assert!(matches!(err, StoreError::Cycle(_)));

        // A goal reused under two parents (a DAG, not a cycle) is allowed.
        let d = goal("d");
        store.upsert_goal(&d).unwrap();
        store
            .add_goal_edge(
                &edge(a.id, ChildRef::Goal(d.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap();
        store
            .add_goal_edge(
                &edge(b.id, ChildRef::Goal(d.id), EdgeRole::Accomplish, 0.0),
                NOW,
            )
            .unwrap();
    }

    #[test]
    fn add_goal_edge_updates_in_place() {
        let mut store = store();
        let parent = goal("g");
        store.upsert_goal(&parent).unwrap();
        let child = ChildRef::Procedure(ProcedureId::new());
        store
            .add_goal_edge(&edge(parent.id, child, EdgeRole::Accomplish, 0.3), NOW)
            .unwrap();
        store
            .add_goal_edge(&edge(parent.id, child, EdgeRole::Accomplish, 0.8), NOW + 1)
            .unwrap();
        let edges = store.get_goal_edges(parent.id).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].cached_effectiveness, 0.8);
    }

    #[test]
    fn recompute_edge_effectiveness_pulls_from_procedure() {
        let mut store = store();
        let parent = goal("cook-eggs");
        store.upsert_goal(&parent).unwrap();

        let mut proc = procedure(ProcedureId::new(), "scramble");
        proc.success_count = 9;
        proc.failure_count = 1;
        proc.last_used_at = Some(NOW);
        store.upsert_procedure(&proc).unwrap();

        // Edge starts with a stale cached effectiveness of 0.0.
        store
            .add_goal_edge(
                &edge(
                    parent.id,
                    ChildRef::Procedure(proc.id),
                    EdgeRole::Accomplish,
                    0.0,
                ),
                NOW,
            )
            .unwrap();

        let updated = store
            .recompute_goal_edge_effectiveness(parent.id, NOW)
            .unwrap();
        assert_eq!(updated, 1);

        let edges = store.get_goal_edges(parent.id).unwrap();
        // success-rate 0.9 * recency 1.0 == 0.9.
        assert!((edges[0].cached_effectiveness - 0.9).abs() < 1e-9);
    }

    #[test]
    fn expand_unknown_goal_is_empty() {
        let store = store();
        let result = store
            .expand(GoalId::new(), &TraversalContext::default(), NOW)
            .unwrap();
        assert!(result.candidates.is_empty());
        assert!(result.decision_aids.is_empty());
        assert!(result.factor_readings.is_empty());
    }

    #[test]
    fn query_goals_by_intent_substring() {
        let mut store = store();
        let mut a = goal("a");
        a.intent = "prepare a quick breakfast".into();
        let mut b = goal("b");
        b.intent = "plan a fancy dinner".into();
        store.upsert_goal(&a).unwrap();
        store.upsert_goal(&b).unwrap();

        let hits = store.query_goals(None, Some("breakfast"), None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, a.id);
    }

    // ---- Graph integrity under decay (design §8, open problem #5) ----

    mod integrity {
        use super::*;

        /// `eat -> prepare-breakfast -> cook-eggs`, plus a procedure leaf under
        /// `cook-eggs`. Returns the three goal ids and the procedure id.
        fn chain(store: &mut SqliteStore) -> (GoalId, GoalId, GoalId, ProcedureId) {
            let eat = goal("eat");
            let breakfast = goal("prepare-breakfast");
            let eggs = goal("cook-eggs");
            let (eat_id, breakfast_id, eggs_id) = (eat.id, breakfast.id, eggs.id);
            store.upsert_goal(&eat).unwrap();
            store.upsert_goal(&breakfast).unwrap();
            store.upsert_goal(&eggs).unwrap();

            let omelette = ProcedureId::new();
            store
                .upsert_procedure(&procedure(omelette, "omelette"))
                .unwrap();

            store
                .add_goal_edge(
                    &edge(
                        eat_id,
                        ChildRef::Goal(breakfast_id),
                        EdgeRole::Accomplish,
                        0.8,
                    ),
                    NOW,
                )
                .unwrap();
            store
                .add_goal_edge(
                    &edge(
                        breakfast_id,
                        ChildRef::Goal(eggs_id),
                        EdgeRole::Accomplish,
                        0.7,
                    ),
                    NOW,
                )
                .unwrap();
            store
                .add_goal_edge(
                    &edge(
                        eggs_id,
                        ChildRef::Procedure(omelette),
                        EdgeRole::Accomplish,
                        0.9,
                    ),
                    NOW,
                )
                .unwrap();

            (eat_id, breakfast_id, eggs_id, omelette)
        }

        /// The failure this whole problem is about: an edge left pointing at a
        /// row that is gone. `expand` would surface it as a healthy-looking
        /// candidate — the edge carries its own cached effectiveness and usage
        /// notes (§4.2) — that cannot be fetched.
        #[test]
        fn a_raw_delete_dangles_the_graph_and_expand_cannot_tell() {
            let mut store = store();
            let (_, _, eggs, omelette) = chain(&mut store);

            // Bypass the collection API entirely, the way a stray DELETE or an
            // older Boswell would.
            store
                .conn
                .execute(
                    "DELETE FROM procedures WHERE id = ?1",
                    params![omelette.value().to_be_bytes().to_vec()],
                )
                .unwrap();

            let surface = store
                .expand(eggs, &TraversalContext::default(), NOW)
                .unwrap();
            assert_eq!(
                surface.candidates.len(),
                1,
                "expand still surfaces the dead child"
            );
            assert_eq!(store.dangling_goal_edges().unwrap().len(), 1);

            // And the repair pass fixes it.
            assert_eq!(store.prune_dangling_goal_edges().unwrap(), 1);
            assert!(store.dangling_goal_edges().unwrap().is_empty());
            let surface = store
                .expand(eggs, &TraversalContext::default(), NOW)
                .unwrap();
            assert!(surface.candidates.is_empty());
        }

        /// Whatever the policy, collecting through the API never leaves a
        /// dangling edge. This is the invariant; the policies differ only in
        /// what they do about orphans.
        #[test]
        fn no_policy_ever_dangles_the_graph() {
            for policy in [
                GraphIntegrity::PinChildren,
                GraphIntegrity::CascadeAndReparent,
                GraphIntegrity::CascadeAndOrphan,
            ] {
                let mut store = store();
                let (_, breakfast, _, omelette) = chain(&mut store);

                store.collect_goal(breakfast, policy, NOW).unwrap();
                store.collect_procedure(omelette, policy).unwrap();

                assert!(
                    store.dangling_goal_edges().unwrap().is_empty(),
                    "{} left a dangling edge",
                    policy.as_str()
                );
            }
        }

        /// `PinChildren` refuses to collect anything an edge points at, and the
        /// refusal is total: the graph is left exactly as it was.
        #[test]
        fn pinning_refuses_and_changes_nothing() {
            let mut store = store();
            let (_, breakfast, eggs, _) = chain(&mut store);

            let outcome = store
                .collect_goal(breakfast, GraphIntegrity::PinChildren, NOW)
                .unwrap();

            assert!(!outcome.collected);
            assert_eq!(outcome.pinned_by, 1);
            assert_eq!(outcome.edges_removed, 0);
            assert!(store.get_goal(breakfast).unwrap().is_some());
            // Its child is still reachable from it.
            assert_eq!(store.get_goal_edges(breakfast).unwrap().len(), 1);
            assert!(store.get_goal(eggs).unwrap().is_some());
        }

        /// The cost of pinning: decay stops at the first reference. Nothing in
        /// the chain below a referenced node can ever be collected, so a single
        /// forgotten root keeps its whole subtree alive — in tension with the
        /// premise that memory fades unless it earns its keep.
        #[test]
        fn pinning_makes_a_referenced_subtree_immortal() {
            let mut store = store();
            let (_, breakfast, eggs, omelette) = chain(&mut store);

            for _ in 0..3 {
                store
                    .collect_goal(breakfast, GraphIntegrity::PinChildren, NOW)
                    .unwrap();
                store
                    .collect_goal(eggs, GraphIntegrity::PinChildren, NOW)
                    .unwrap();
                store
                    .collect_procedure(omelette, GraphIntegrity::PinChildren)
                    .unwrap();
            }

            assert!(store.get_goal(breakfast).unwrap().is_some());
            assert!(store.get_goal(eggs).unwrap().is_some());
            assert!(store.get_procedure(omelette).unwrap().is_some());
        }

        /// A root nothing points at is collectable even under pinning — pinning
        /// blocks on *references*, not on having children.
        #[test]
        fn pinning_still_collects_an_unreferenced_root() {
            let mut store = store();
            let (eat, breakfast, _, _) = chain(&mut store);

            let outcome = store
                .collect_goal(eat, GraphIntegrity::PinChildren, NOW)
                .unwrap();

            assert!(outcome.collected);
            assert_eq!(outcome.pinned_by, 0);
            assert!(store.get_goal(eat).unwrap().is_none());
            // Its child survives, now orphaned.
            assert!(store.get_goal(breakfast).unwrap().is_some());
            assert_eq!(outcome.children_orphaned, 1);
        }

        /// `CascadeAndReparent` keeps the child reachable by re-attaching it to
        /// the collected node's parent.
        #[test]
        fn reparenting_keeps_a_grandchild_reachable() {
            let mut store = store();
            let (eat, breakfast, eggs, _) = chain(&mut store);

            let outcome = store
                .collect_goal(breakfast, GraphIntegrity::CascadeAndReparent, NOW)
                .unwrap();

            assert!(outcome.collected);
            assert_eq!(outcome.children_reparented, 1);
            assert_eq!(outcome.children_orphaned, 0);

            // `cook-eggs` now hangs directly under `eat`.
            let surface = store
                .expand(eat, &TraversalContext::default(), NOW)
                .unwrap();
            assert_eq!(surface.candidates.len(), 1);
            assert_eq!(surface.candidates[0].child, ChildRef::Goal(eggs));
        }

        /// The cost of re-parenting: the edge it writes is **fabricated**. Edge
        /// payload is placement-specific (§3.2 — "prefer eggs here when LDL is
        /// low"), so carrying it to a grandparent asserts about that placement
        /// something nobody ever said.
        #[test]
        fn reparenting_fabricates_placement_specific_context() {
            let mut store = store();
            let (eat, breakfast, eggs, _) = chain(&mut store);

            // Give the breakfast -> eggs edge context that is only true *under
            // breakfast*.
            let mut contextual = edge(breakfast, ChildRef::Goal(eggs), EdgeRole::Accomplish, 0.7);
            contextual.context_tags = vec!["meal:breakfast".into()];
            contextual.usage_notes = "the obvious choice first thing in the morning".into();
            store.add_goal_edge(&contextual, NOW).unwrap();

            store
                .collect_goal(breakfast, GraphIntegrity::CascadeAndReparent, NOW)
                .unwrap();

            let moved = store.get_goal_edges(eat).unwrap();
            let moved = moved
                .iter()
                .find(|e| e.child == ChildRef::Goal(eggs))
                .expect("re-parented onto eat");

            // Now attached to `eat`, still claiming to be about breakfast.
            assert_eq!(moved.context_tags, vec!["meal:breakfast".to_string()]);
            assert_eq!(
                moved.usage_notes,
                "the obvious choice first thing in the morning"
            );
        }

        /// Re-parenting cannot close a cycle, and the reason is structural
        /// rather than lucky: the new edge `ancestor -> child` is a **shortcut**
        /// over the path `ancestor -> node -> child` that already existed. For
        /// it to close a cycle the graph would need `child ->* ancestor` too,
        /// which the write-time guard already refuses.
        ///
        /// This test pins both halves — that such a graph cannot be built, and
        /// that re-parenting across a diamond stays acyclic. `collect_goal`
        /// still handles [`StoreError::Cycle`] defensively, because a graph
        /// damaged outside the API (a raw `DELETE`, a restored backup) carries
        /// no such guarantee.
        #[test]
        fn reparenting_is_a_shortcut_and_cannot_close_a_cycle() {
            // The cycle re-parenting would need is unbuildable in the first place.
            let mut back_edge = store();
            let a = goal("a");
            let b = goal("b");
            let (a_id, b_id) = (a.id, b.id);
            back_edge.upsert_goal(&a).unwrap();
            back_edge.upsert_goal(&b).unwrap();
            back_edge
                .add_goal_edge(
                    &edge(a_id, ChildRef::Goal(b_id), EdgeRole::Accomplish, 0.5),
                    NOW,
                )
                .unwrap();
            assert!(matches!(
                back_edge.add_goal_edge(
                    &edge(b_id, ChildRef::Goal(a_id), EdgeRole::Accomplish, 0.5),
                    NOW
                ),
                Err(StoreError::Cycle(_))
            ));

            // A diamond: eat reaches eggs two ways. Collecting one arm
            // re-parents onto a node that already reaches the child.
            let mut store = store();
            let eat = goal("eat");
            let breakfast = goal("prepare-breakfast");
            let dinner = goal("quick-dinner");
            let eggs = goal("cook-eggs");
            let (eat_id, breakfast_id, dinner_id, eggs_id) =
                (eat.id, breakfast.id, dinner.id, eggs.id);
            for g in [&eat, &breakfast, &dinner, &eggs] {
                store.upsert_goal(g).unwrap();
            }
            for (parent, child) in [
                (eat_id, breakfast_id),
                (eat_id, dinner_id),
                (breakfast_id, eggs_id),
                (dinner_id, eggs_id),
            ] {
                store
                    .add_goal_edge(
                        &edge(parent, ChildRef::Goal(child), EdgeRole::Accomplish, 0.5),
                        NOW,
                    )
                    .unwrap();
            }

            let outcome = store
                .collect_goal(breakfast_id, GraphIntegrity::CascadeAndReparent, NOW)
                .unwrap();

            assert!(outcome.collected);
            assert_eq!(outcome.children_reparented, 1);
            assert!(store.dangling_goal_edges().unwrap().is_empty());

            // eggs is reachable from eat both directly and via quick-dinner, and
            // the graph is still acyclic — adding the back edge is still refused.
            assert!(matches!(
                store.add_goal_edge(
                    &edge(eggs_id, ChildRef::Goal(eat_id), EdgeRole::Accomplish, 0.5),
                    NOW
                ),
                Err(StoreError::Cycle(_))
            ));
            assert!(store.get_goal(eggs_id).unwrap().is_some());
        }

        /// `CascadeAndOrphan` lets decay proceed and invents nothing: the child
        /// row survives untouched, it is simply no longer reachable by descent.
        #[test]
        fn orphaning_collects_without_inventing_structure() {
            let mut store = store();
            let (eat, breakfast, eggs, _) = chain(&mut store);

            let outcome = store
                .collect_goal(breakfast, GraphIntegrity::CascadeAndOrphan, NOW)
                .unwrap();

            assert!(outcome.collected);
            assert_eq!(outcome.children_reparented, 0);
            assert_eq!(outcome.children_orphaned, 1);

            // Nothing was written onto `eat`.
            assert!(store
                .expand(eat, &TraversalContext::default(), NOW)
                .unwrap()
                .candidates
                .is_empty());

            // But `cook-eggs` itself is intact and still findable by query.
            let eggs_row = store.get_goal(eggs).unwrap().expect("child survives");
            assert_eq!(eggs_row.name, "cook-eggs");
            assert!(store.orphaned_goals().unwrap().iter().any(|g| g.id == eggs));
        }

        /// Collecting a procedure removes the edges that pointed at it, so a
        /// parent goal simply stops surfacing it.
        #[test]
        fn collecting_a_leaf_procedure_unhooks_it_from_its_parent() {
            let mut store = store();
            let (_, _, eggs, omelette) = chain(&mut store);

            let outcome = store
                .collect_procedure(omelette, GraphIntegrity::CascadeAndOrphan)
                .unwrap();

            assert!(outcome.collected);
            assert_eq!(outcome.edges_removed, 1);
            assert!(store
                .expand(eggs, &TraversalContext::default(), NOW)
                .unwrap()
                .candidates
                .is_empty());
            assert!(store.dangling_goal_edges().unwrap().is_empty());
        }

        /// Collecting something that is not there is a no-op, not an error, so a
        /// sweep can be re-run safely.
        #[test]
        fn collecting_a_missing_node_is_a_no_op() {
            let mut store = store();
            let outcome = store
                .collect_goal(GoalId::new(), GraphIntegrity::CascadeAndOrphan, NOW)
                .unwrap();
            assert!(!outcome.collected);
            assert_eq!(outcome.edges_removed, 0);

            let outcome = store
                .collect_procedure(ProcedureId::new(), GraphIntegrity::CascadeAndOrphan)
                .unwrap();
            assert!(!outcome.collected);
        }

        /// A procedure's outstanding receipts survive it: a receipt is an
        /// obligation someone already took on, and the report path already
        /// handles a procedure that has since gone.
        #[test]
        fn collecting_a_procedure_leaves_its_receipts_answerable() {
            use boswell_domain::ExecutionReceipt;

            let mut store = store();
            let (_, _, _, omelette) = chain(&mut store);
            let p = store.get_procedure(omelette).unwrap().unwrap();
            let receipt = ExecutionReceipt::issue(&p, "agent:cook", NOW, 60_000);
            let receipt_id = receipt.receipt_id;
            store.issue_receipt(&receipt).unwrap();

            store
                .collect_procedure(omelette, GraphIntegrity::CascadeAndOrphan)
                .unwrap();

            assert!(
                store.get_receipt(receipt_id).unwrap().is_some(),
                "the obligation outlives the procedure"
            );
        }

        #[test]
        fn policy_names_round_trip() {
            for p in [
                GraphIntegrity::PinChildren,
                GraphIntegrity::CascadeAndReparent,
                GraphIntegrity::CascadeAndOrphan,
            ] {
                assert_eq!(GraphIntegrity::parse(p.as_str()), Some(p));
            }
            assert!(GraphIntegrity::parse("nope").is_none());
            assert_eq!(GraphIntegrity::default(), GraphIntegrity::CascadeAndOrphan);
        }
    }
}
