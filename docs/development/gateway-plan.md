# Boswell public HTTP API gateway (`boswell-gateway`) — implementation plan

## Context

Boswell today is reachable only by a local process: all memory operations are
**gRPC-only** (`BosWellService`), wrapped by the in-repo `boswell-sdk` `BoswellClient`,
which does the `/session/establish` → gRPC handshake. The only HTTP that exists is the
Router's topology surface (`/session/establish`, `/health`). Auth is a stub end to end
(the instance only checks `auth_token` is non-empty; the Router's HS256 JWT is never
validated; no TLS anywhere); the mTLS/Ed25519 model in the ADRs is aspirational.

Boswell's primary consumers are **cloud agents** (e.g. Claude on iPad), which cannot
reach a localhost-only, gRPC/CLI-based Boswell. This plan adds a **public,
authenticated HTTP/JSON API** — a new `boswell-gateway` crate — exposing the full memory
lifecycle (read, write, search, recall, delete, relationships, and hook ingest incl. LLM
extraction) so any external agent can use Boswell over HTTPS. It turns the public-serving
design in `docs/integrations/claude-code-hooks.md` into real code.

## Confirmed decisions

- New crate `crates/boswell-gateway/`; it **reuses the in-repo `boswell-sdk`**
  `BoswellClient` internally (no external/Riptide SDK — this repo is independent of
  Riptide).
- Auth: **static bearer API keys** → namespace + scope. `Authorization: Bearer <key>`;
  keys stored as **hashes** in gateway config, each mapped to `{ namespace, scopes }`.
- TLS/public reach via **reverse proxy or tunnel**; the gateway serves plain HTTP on
  localhost. The gRPC instance stays private.
- Build the **full suite now**, including new gRPC RPCs and server-side LLM extraction.

## Grounded facts (verified — do not re-derive)

- SQLite store `crates/boswell-store/src/lib.rs` fully implements `get_claim` (~304),
  `query_claims` (~338; filters namespace/tier/min_confidence/limit — subject/predicate/
  object are filtered in-memory in the gRPC service; `source_type` column + index exist
  but are not a filter yet), `add_relationship` (~399), `get_relationships` (~421),
  `delete_claim` (~495, real cascading DELETE), `update_claim_tier` (~506).
- gRPC service `crates/boswell-grpc/src/service.rs`: auth is a stub (only checks
  `auth_token` non-empty at ~48/104/178/239/285); `forget` (~278) is a
  "marked for eviction (stub)" that does **not** delete. No TLS (`server.rs`
  `enable_tls` is a `println!` placeholder).
- Proto `crates/boswell-grpc/proto/boswell.proto`, service `BosWellService`: Assert,
  Query, Search, Learn, Forget, HealthCheck. `QueryFilter` = namespace/subject/predicate/
  object/tier/min_confidence. `Claim` = id/namespace/subject/predicate/object/
  confidence{lower,upper}/tier/source_type.
- SDK `crates/boswell-sdk/src/client.rs`: `assert`, `query(QueryFilter)`, `search`,
  `learn`, `forget`; `connect()` does session-establish then gRPC; each method attaches
  `auth_token` and reconnects once on `Unauthenticated`.
- Domain `RelationshipType` = `Supports|Contradicts|DerivedFrom|References|Supersedes`
  ("corroborates" == `Supports`). Tiers = `ephemeral|task|project|permanent`.
- Extractor `crates/boswell-extractor/src/extractor.rs`: `extract(ExtractionRequest) ->
  ExtractionResult` (LLM → parse → Gatekeeper validate → `store.assert`). It owns its
  store by value today; for server-side use add a shared-store path mirroring
  `Synthesizer::run_pass_shared(Arc<Mutex<S>>)` (`crates/boswell-synthesizer/src/synthesizer.rs:123`).
  Emitted claims get `source_type="extraction"`.
- MCP tier-enum bug: `crates/boswell-mcp/src/server.rs` advertises
  `Transient|Session|Permanent`; must be `ephemeral|task|project|permanent`.
- Claim JSON DTO to mirror (`boswell-cli/src/output.rs`): `{id, namespace, subject,
  predicate, object, confidence:{lower,upper}, tier, source_type, created_at, stale_at}`.

## Architecture

```
Cloud agent ──HTTPS──> [reverse proxy / tunnel: TLS] ──HTTP──> boswell-gateway (axum)
                                                                   │ API-key auth → namespace/scope
                                                                   │ reuses boswell-sdk BoswellClient
                                                                   ▼
                                                boswell-router (session) + boswell-server (gRPC, private)
```

## Endpoint suite (all under `/v1`, JSON)

| Method & path | Purpose | Backing |
|---|---|---|
| `GET /v1/health` | liveness + instance health | `HealthCheck` (unauthenticated) |
| `POST /v1/claims` | assert one claim | SDK `assert` → `Assert` |
| `POST /v1/claims/batch` | bulk learn | SDK `learn` → `Learn` |
| `GET /v1/claims` | query (namespace, subject, predicate, object, tier, min_confidence, **source_type**, limit) | SDK `query` → `Query` |
| `GET /v1/claims/{id}` | fetch one claim | **new** `GetClaim` RPC → `store.get_claim` |
| `GET /v1/claims/{id}/relationships` | provenance/contradiction graph | **new** `GetRelationships` RPC → `store.get_relationships` |
| `DELETE /v1/claims/{id}` | forget (**real delete**) | `Forget` rewired → `store.delete_claim` |
| `POST /v1/search` | semantic search (query, namespace?, limit, min_similarity) | SDK `search` → `Search` |
| `POST /v1/recall` | one-call context for a subject/topic: merge `query`(subject) + `search`(text), ranked | composition over `Query`+`Search` |
| `POST /v1/extract` | text → claims via LLM Extractor | **new** `Extract` RPC (server-side Extractor) |
| `POST /v1/hooks/ingest` | Claude Code hook event JSON → claims (deterministic → `Learn`; LLM mode → `Extract`) | `Learn` and/or `Extract` |

## Cross-crate changes & build order

1. **Proto** (`boswell-grpc/proto/boswell.proto`): add RPCs `GetClaim`,
   `GetRelationships`, `Extract`; add `Relationship` message + `RelationshipType` enum
   (mirror domain); add `optional string source_type` to `QueryFilter`; new requests
   carry `auth_token`. Regen; extend `conversions.rs` (add `relationship_to_proto`).
2. **Store/domain filter**: add `source_type: Option<String>` to `ClaimQuery`
   (`boswell-domain/src/traits.rs`) and `AND source_type = ?` in `query_claims`.
3. **gRPC service** (`service.rs`): implement `get_claim`, `get_relationships`; **rewire
   `forget` → `store.delete_claim`**; push `source_type` into the query.
4. **SDK** (`boswell-sdk/src/client.rs`): add `get_claim`, `get_relationships`,
   `extract`; add `source_type` to the SDK `QueryFilter`; make `forget` reflect real
   deletion. Keep the per-method `auth_token` + reconnect-once pattern.
5. **New `crates/boswell-gateway/`**: `main.rs`/`lib.rs` (axum, shared `BoswellClient`,
   default bind `127.0.0.1:8081`); `config.rs` + `config/gateway.toml`
   (`bind_address`, `bind_port`, `router_endpoint`, `[[api_keys]]` = `{ id, key_hash,
   namespace, scopes }`); `auth.rs` middleware (bearer → `AuthContext{key_id, namespace,
   scopes}`; enforce scope on writes; `/v1/health` bypass); `handlers/*.rs` per resource;
   uniform JSON error `{error, request_id}`; `tower_http` body-limit + timeout + trace;
   per-key rate limit; per-mutation audit log line. Follow the axum patterns in
   `crates/boswell-router/src/handlers.rs`. Add the crate to workspace `Cargo.toml`.
6. **Extractor over gRPC** (highest-risk): add an `Arc<Mutex<S>>` shared-store extraction
   path to the Extractor (model on `Synthesizer::run_pass_shared`); give
   `BosWellServiceImpl` an optional extractor/LLM handle set at server startup in
   `boswell-server`; implement the `Extract` RPC; wire gateway `/v1/extract` and
   LLM-mode `/v1/hooks/ingest`. Deterministic ingest (via `Learn`) must keep working if
   the LLM path is unavailable.
7. **Consistency + docs**: fix the MCP tier enum (`boswell-mcp/src/server.rs`); add
   `docs/integrations/http-api.md` (endpoints, auth + scopes, claim DTO, deploy via
   proxy/tunnel); update `docs/integrations/claude-code-hooks.md` +
   `examples/claude-code-hooks/settings.example.json` now that `/v1/hooks/ingest` is
   real; note the gateway in `README.md`.

## Verification

- `cargo build`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test` across
  the workspace (CI runs exactly these — `.github/workflows/ci.yml`).
- Unit tests: gateway auth middleware (missing/invalid/out-of-scope key → 401/403; valid
  → 200); handlers against a mock `BoswellClient`; new service RPCs against an in-memory
  store (assert→get_claim; add_relationship→get_relationships; assert→forget→get_claim
  returns None; query by source_type).
- End-to-end (mock embeddings, no Ollama): `backend="mock"` in `config/instance.toml`;
  run `boswell-server` + `boswell-router` + `boswell-gateway`; `curl`:
  `POST /v1/claims`→`GET /v1/claims/{id}` round-trip; `GET /v1/claims?source_type=inference`;
  `DELETE /v1/claims/{id}`→`GET` 404; `POST /v1/recall`; auth rejection (no/short/
  out-of-namespace key) vs a scoped key succeeding only in its namespace.
- LLM path (Ollama up): `POST /v1/extract` and LLM-mode `/v1/hooks/ingest` create
  `source_type=extraction` claims retrievable via `GET /v1/claims`.
- Confirm the gateway binds localhost only and no plaintext API keys are logged or stored
  (hashes only).

## Conventions

- Branch: `claude/boswell-gateway-http-api` (cut from `main`). Open a PR against `main`
  only when the owner asks.
- End commit messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
  Keep any model identifier out of committed artifacts (chat only).
