# Boswell — API Surface

This document specifies every operation in the Boswell API. All operations are exposed via gRPC and are available through the MCP server, client SDKs, and direct gRPC clients. The session protocol is described first, followed by each operation.

## Transport

All communication uses gRPC over mTLS. Every connection requires mutual certificate verification. There are no unauthenticated endpoints.

Streaming is supported for operations that may produce incremental results (Reflect, deliberate Query). The agent receives results as they become available rather than waiting for the complete response.

## Session Protocol

Sessions are topology discovery handshakes. They establish authentication and provide the client with the instance topology for direct routing.

### SessionRequest

Empty beyond mTLS identity. No operational context.

```protobuf
message SessionRequest {}
```

### SessionResponse

```protobuf
message SessionResponse {
  string token = 1;                    // Session token for subsequent operations
  string mode = 2;                     // "router" or "instance"
  repeated InstanceInfo instances = 3; // Available instances with endpoints and expertise
}

message InstanceInfo {
  string instance_id = 1;
  string endpoint = 2;
  repeated string expertise = 3;       // Topic descriptors for client-side routing
  string health = 4;                   // "healthy", "degraded", "unreachable"
}
```

**Behavior:**

- In single-instance mode: returns one entry in the instances array with the instance's own endpoint. Expertise may be empty. Mode is `"instance"`.
- In multi-instance mode (via Router): returns all registered instances with their endpoints, expertise profiles, and current health states. Mode is `"router"`.
- The token is valid for all instances listed in the response. Instances accept the token based on the issuing authority's signing key.
- The client SDK uses the expertise profiles to route operations directly to instances. The Router is a fallback for ambiguous routing.

### Token Lifecycle

- Tokens are short-lived. Expiration is configurable (default: 1 hour).
- The client can re-issue a SessionRequest to refresh the token and receive an updated topology.
- Expired tokens are rejected with an `UNAUTHENTICATED` status. The client must re-authenticate.

---

## Operations

### Assert

Submit one or more claims with provenance.

**Request:**

```protobuf
message AssertRequest {
  string token = 1;
  repeated ClaimInput claims = 2;
  string namespace = 3;            // Target namespace (applied to all claims in batch unless overridden per-claim)
  string tier = 4;                 // Target tier: "ephemeral", "task", "project", "persistent"
  string routing_hint = 5;         // Optional domain hint for Router classification (transient, not persisted)
}

message ClaimInput {
  string subject = 1;
  string predicate = 2;
  string direct_object = 3;
  string raw_expression = 4;
  ProvenanceInput provenance = 5;  // Source information for this assertion
  string namespace = 6;            // Per-claim namespace override (optional)
  string tier = 7;                 // Per-claim tier override (optional)
  string routing_hint = 8;         // Per-claim routing hint override (optional)
}

message ProvenanceInput {
  string source_type = 1;          // "agent_assertion", "user_input", "direct_load"
  string source_id = 2;            // Identifier for the asserting agent or user
  float confidence_contribution = 3;
  string context = 4;              // Free-form notes on why/how
}
```

**Response:**

```protobuf
message AssertResponse {
  repeated ClaimResult results = 1;
}

message ClaimResult {
  string claim_id = 1;            // ULID of the created or updated claim
  string status = 2;              // "created", "corroborated", "failed"
  string instance = 3;            // Instance that received this claim
  string reason = 4;              // Populated on failure (e.g., "instance_unreachable", "validation_error")
}
```

**Behavior:**

- Accepts batches of one or more claims.
- Each claim is embedded (via the instance's configured local embedding model) and stored in both SQLite and the HNSW vector index.
- **Duplicate detection:** If a semantically identical claim already exists (determined by embedding similarity above a configurable threshold), the existing claim receives a new provenance entry (corroboration) rather than creating a duplicate. The response status is `"corroborated"` with the existing claim's ID.
- **Partial success:** In multi-instance deployments where a batch spans instances, each sub-batch succeeds or fails independently. The response reports per-claim status.
- **Validation errors:** Claims with invalid namespaces (exceeding max depth), missing required fields, or other structural problems are rejected individually without affecting other claims in the batch.
- Namespace and tier from the batch-level fields apply to all claims unless overridden per-claim.

**Error conditions:**

| gRPC Status | Condition |
|---|---|
| `UNAUTHENTICATED` | Token expired or invalid |
| `INVALID_ARGUMENT` | Missing required fields, invalid namespace depth, invalid tier value |
| `UNAVAILABLE` | Target instance unreachable (partial failure in multi-instance) |
| `RESOURCE_EXHAUSTED` | Instance at capacity or rate-limited |

---

### Query

Retrieve claims by structure, semantics, or time.

**Request:**

```protobuf
message QueryRequest {
  string token = 1;

  // Structural query (point lookup, filtered search)
  string subject = 2;
  string predicate = 3;
  string direct_object = 4;

  // Semantic query (nearest-neighbor)
  string semantic_query = 5;       // Natural language query text, embedded at query time
  int32 semantic_limit = 6;        // Max results for semantic search (default: 10)
  float similarity_threshold = 7;  // Minimum similarity score (default: 0.0)

  // Temporal query
  string since = 8;                // ISO 8601 datetime — claims modified after this time
  string until = 9;                // ISO 8601 datetime — claims modified before this time

  // Filters (apply to all query modes)
  string namespace = 10;           // Exact, recursive ("ns/*"), or depth-limited ("ns/*/1")
  repeated string tiers = 11;      // Filter by tier(s)
  repeated string statuses = 12;   // Filter by status(es) (default: ["active"])
  float min_confidence_lower = 13; // Minimum lower bound of confidence interval
  float min_confidence_upper = 14; // Minimum upper bound of confidence interval

  // Mode
  bool deliberate = 15;            // If true, use LLM-assisted evaluation (slower, richer)
  string routing_hint = 16;        // Optional domain hint for routing
}
```

**Response:**

```protobuf
message QueryResponse {
  repeated ClaimOutput claims = 1;
  string narrative = 2;            // Populated only in deliberate mode — synthesized assessment
  QueryCoverage coverage = 3;      // In federated queries, reports which instances responded
}

message ClaimOutput {
  string claim_id = 1;
  string subject = 2;
  string predicate = 3;
  string direct_object = 4;
  string raw_expression = 5;
  ConfidenceInterval confidence = 6;
  repeated ProvenanceOutput provenance = 7;
  ScopeOutput scope = 8;
  LifecycleOutput lifecycle = 9;
  repeated RelationshipOutput relationships = 10;
  float relevance_score = 11;      // Populated for semantic queries — similarity to query
}

message ConfidenceInterval {
  float lower_bound = 1;
  float upper_bound = 2;
  string computation_log = 3;      // Populated in deliberate mode
}

message QueryCoverage {
  int32 instances_queried = 1;
  int32 instances_responded = 2;
  repeated string unreachable_instances = 3;
}
```

**Behavior:**

- **Structural queries** use SQLite indexes for direct lookup. Fields are ANDed — providing subject and predicate returns claims matching both.
- **Semantic queries** embed the query text using the local embedding model, search the HNSW index for nearest neighbors, then resolve full claim data from SQLite.
- **Temporal queries** use ULID range scans for efficient time-bounded retrieval.
- Query modes can be combined. A semantic query with a namespace filter searches semantically within that namespace.
- **Fast mode (default):** Confidence intervals are computed deterministically from cached values. Sub-millisecond for point lookups, low milliseconds for semantic search.
- **Deliberate mode:** Invokes LLM to evaluate claim confidence in the context of the specific query. Returns a narrative alongside results. Same claim may receive different confidence treatment depending on what the agent is asking.
- **Namespace query modes:** Exact match (`"acme/project"`), recursive (`"acme/project/*"`), depth-limited (`"acme/project/*/1"`).

**Error conditions:**

| gRPC Status | Condition |
|---|---|
| `UNAUTHENTICATED` | Token expired or invalid |
| `INVALID_ARGUMENT` | Invalid namespace pattern, invalid tier/status values |
| `DEADLINE_EXCEEDED` | Deliberate mode LLM evaluation timed out |

---

### Challenge

Register a contradiction against an existing claim.

**Request:**

```protobuf
message ChallengeRequest {
  string token = 1;
  string target_claim_id = 2;      // The claim being challenged
  string challenging_claim_id = 3; // The claim that contradicts it (optional — may be a new assertion)
  string raw_expression = 4;       // Natural language description of the contradiction
  string evidence = 5;             // Supporting context for the challenge
  string source_id = 6;            // Identifier of the challenging agent
  string routing_hint = 7;
}
```

**Response:**

```protobuf
message ChallengeResponse {
  string challenge_id = 1;         // ID of the created contradiction relationship
  string target_status = 2;        // Updated status of the target claim ("active" or "challenged")
}
```

**Behavior:**

- Creates a `contradicts` relationship between the challenging claim and the target claim.
- If the target claim was `active`, its status transitions to `challenged`.
- If `challenging_claim_id` is not provided, the system creates a new claim from the `raw_expression` with `source_type: agent_assertion` and uses it as the challenger.
- The challenge is recorded as a provenance entry on the target claim for audit trail.
- Contradiction resolution is handled by the Janitor's LLM-assisted contradiction detection or manually via subsequent Assert/Challenge operations.

---

### Promote / Demote

Request tier migration for one or more claims.

**Request:**

```protobuf
message PromoteRequest {
  string token = 1;
  repeated PromotionCandidate candidates = 2;
  string routing_hint = 3;
}

message PromotionCandidate {
  string claim_id = 1;
  string target_tier = 2;          // Desired tier
  float perceived_importance = 3;  // Agent's advocacy: how important (0.0-1.0)
  float advocacy_confidence = 4;   // Agent's advocacy: how confident in that assessment (0.0-1.0)
  string justification = 5;        // Free-form reasoning for the promotion
}
```

**Response:**

```protobuf
message PromoteResponse {
  repeated PromotionResult results = 1;
}

message PromotionResult {
  string claim_id = 1;
  string status = 2;               // "promoted", "rejected", "deferred"
  string previous_tier = 3;
  string current_tier = 4;
  string reasoning = 5;            // Gatekeeper's evaluation reasoning
}
```

**Behavior:**

- **Promotion (tier increase):** Triggers Gatekeeper evaluation. The Gatekeeper is not bound by the agent's advocacy — it evaluates independently against existing knowledge at the target tier.
- **Demotion (tier decrease):** Does not require Gatekeeper evaluation. Processed directly. Claims can always be moved to a lower tier.
- Rejected claims remain at their current tier with their existing TTL. The Gatekeeper's reasoning is recorded as a provenance entry.
- `"deferred"` status indicates the Gatekeeper needs more time (e.g., LLM provider is slow). The client can poll.

---

### Extract

Submit a text block for claim extraction.

**Request:**

```protobuf
message ExtractRequest {
  string token = 1;
  string text = 2;                 // The text block to extract claims from
  string namespace = 3;            // Target namespace for extracted claims
  string tier = 4;                 // Target tier for extracted claims
  string source_id = 5;            // Identifier for the source (e.g., document hash, URL)
  string routing_hint = 6;
}
```

**Response:**

```protobuf
message ExtractResponse {
  repeated ClaimResult results = 1; // Extracted claims with IDs and status
  int32 claims_created = 2;
  int32 claims_corroborated = 3;   // Existing claims that matched extracted content
}
```

**Behavior:**

- **Synchronous blocking operation.** The call blocks until all claims are extracted. There is no partial or streaming result model.
- The text block is sent to the configured LLM provider (Extractor subsystem) with instructions to produce discrete claims in Boswell's format.
- Each extracted claim is checked for semantic duplicates. Duplicates become corroboration (new provenance entry on existing claim).
- All extracted claims carry `source_type: extraction` with the `source_id` for traceability.
- This is the most expensive operation in the API due to LLM involvement. Rate limiting is recommended.

---

### Learn

Bulk-load pre-formatted claims directly into the Claim Store.

**Request:**

```protobuf
message LearnRequest {
  string token = 1;
  repeated ClaimInput claims = 2;  // Pre-formatted claims (same structure as Assert)
  float trust_level = 3;           // Initial confidence for all loaded claims (0.0-1.0)
  string conflict_policy = 4;      // "flag", "quiet", "reject" — how to handle contradictions
  string namespace = 5;
  string tier = 6;
  string routing_hint = 7;
}
```

**Response:**

```protobuf
message LearnResponse {
  repeated ClaimResult results = 1;
  int32 claims_loaded = 2;
  int32 claims_conflicted = 3;
  int32 claims_rejected = 4;
}
```

**Behavior:**

- No LLM involvement. Claims are loaded directly with `source_type: direct_load` provenance.
- Embeddings are computed locally for each claim.
- **Conflict policies:**
  - `"flag"`: Load claims and immediately create `contradicts` relationships with existing conflicting claims. Both claims remain active.
  - `"quiet"`: Load claims silently. The Janitor will detect contradictions on its next pass.
  - `"reject"`: Check for conflicts before loading. Reject claims that contradict existing claims with higher confidence.
- **Use cases:** Restoring from export, loading curated knowledge bases, importing from another Boswell instance, bootstrapping domain knowledge.
- Supports batches. Same partial success model as Assert.

---

### Reflect

Request a synthesized summary of knowledge about a topic.

**Request:**

```protobuf
message ReflectRequest {
  string token = 1;
  string topic = 2;               // Natural language topic to reflect on
  string namespace = 3;           // Scope the reflection (optional)
  int32 depth = 4;                // How many relationship hops to follow (default: 2)
  string routing_hint = 5;
}
```

**Response (streaming):**

```protobuf
message ReflectResponse {
  string narrative = 1;            // Synthesized summary (may arrive in chunks via streaming)
  repeated ClaimOutput supporting_claims = 2; // Claims that informed the reflection
  repeated string weak_spots = 3;  // Areas of low confidence or wide intervals identified
  repeated string contradictions = 4; // Unresolved tensions in the knowledge
}
```

**Behavior:**

- **LLM-backed.** Queries relevant claims semantically, then invokes the LLM to synthesize a narrative.
- **Streaming.** The narrative may be streamed as the LLM generates it. Supporting claims are sent after the narrative is complete.
- Identifies weak spots (claims with wide confidence intervals) and unresolved contradictions as part of the reflection.
- The `depth` parameter controls how far to follow relationship chains when gathering context.
- This is not retrieval — it's synthesis. The result is a new perspective on existing knowledge, not a list of claims.

---

### Forget

Request eviction of a claim.

**Request:**

```protobuf
message ForgetRequest {
  string token = 1;
  repeated string claim_ids = 2;   // Claims to forget
  string routing_hint = 3;
}
```

**Response:**

```protobuf
message ForgetResponse {
  repeated ForgetResult results = 1;
}

message ForgetResult {
  string claim_id = 1;
  string status = 2;               // "forgotten", "already_forgotten", "not_found"
}
```

**Behavior:**

- Transitions claims to `forgotten` status. This is a logical operation, not a physical delete.
- Forgotten claims are no longer returned by Query (unless explicitly filtered for `status: "forgotten"`).
- The GC Janitor will hard-delete forgotten claims after a configurable retention period.
- Forgetting a claim does not cascade. Related claims are unaffected, though their confidence may be recomputed by the Janitor if a supporting relationship was removed.
- Supports batches.

---

## Error Model

All errors use standard gRPC status codes with structured error details in the response metadata.

```protobuf
message ErrorDetail {
  string code = 1;        // Machine-readable error code (e.g., "NAMESPACE_TOO_DEEP")
  string message = 2;     // Human-readable description
  string claim_id = 3;    // If the error relates to a specific claim in a batch
  string field = 4;       // If the error relates to a specific request field
}
```

**Common error codes:**

| Code | Description |
|---|---|
| `NAMESPACE_TOO_DEEP` | Namespace exceeds configured maximum depth |
| `INVALID_TIER` | Tier value not in allowed enum |
| `CLAIM_NOT_FOUND` | Referenced claim ID does not exist |
| `DUPLICATE_CHALLENGE` | A contradiction relationship already exists between these claims |
| `INSTANCE_UNREACHABLE` | Target instance could not be reached (multi-instance) |
| `EXTRACTION_FAILED` | LLM provider failed during Extract |
| `GATEKEEPER_TIMEOUT` | Gatekeeper LLM evaluation timed out |
| `RATE_LIMITED` | Too many requests; back off and retry |

## Idempotency

- **Assert:** Idempotent via duplicate detection. Reasserting the same claim adds corroboration.
- **Challenge:** Not idempotent. Duplicate challenges are rejected with `DUPLICATE_CHALLENGE`.
- **Forget:** Idempotent. Forgetting an already-forgotten claim returns `"already_forgotten"`.
- **Extract:** Not idempotent. Re-extracting the same text produces corroboration for existing claims.
- **Learn:** Same idempotency behavior as Assert (duplicate detection applies).

## Rate Limiting

Rate limiting is configurable per operation per instance. Recommended defaults:

| Operation | Default Limit | Rationale |
|---|---|---|
| Assert | 100 batches/minute | Bounded by embedding computation |
| Query (fast) | 1000/minute | Lightweight, primarily index lookups |
| Query (deliberate) | 10/minute | LLM-backed, expensive |
| Extract | 5/minute | Heavy LLM involvement |
| Learn | 20 batches/minute | Embedding-bound like Assert, typically larger batches |
| Reflect | 10/minute | LLM-backed |

Exceeded limits return `RESOURCE_EXHAUSTED` with a `Retry-After` header indicating when to retry.
