# Boswell HTTP API (`boswell-gateway`)

The gateway is a public, authenticated HTTP/JSON front end over the full Boswell
memory lifecycle. It exists so that agents which cannot reach a localhost-only,
gRPC-based Boswell — cloud agents, Claude on the web, a teammate's machine — can
read and write memory over HTTPS.

Internally the gateway reuses the in-repo SDK (`BoswellClient`): it establishes a
Router session and calls the private gRPC instance. The instance's gRPC port
stays bound to `127.0.0.1`; only the gateway is exposed, and only through a
reverse proxy or tunnel that terminates TLS.

```
Cloud agent ──HTTPS──> [reverse proxy / tunnel: TLS] ──HTTP──> boswell-gateway
                                                                   │ API-key auth → namespace/scope
                                                                   │ reuses boswell-sdk BoswellClient
                                                                   ▼
                                              boswell-router (session) + boswell-server (gRPC, private)
```

## Running

```bash
# Write a starter config, then edit it to add API-key hashes.
cargo run -p boswell-gateway -- init config/gateway.toml
cargo run -p boswell-gateway -- --config config/gateway.toml
```

The gateway serves plain HTTP on `127.0.0.1:8081` by default. It is meant to sit
behind TLS termination — see [Deployment](#deployment).

### Configuration

```toml
bind_address = "127.0.0.1"
bind_port = 8081
router_endpoint = "http://127.0.0.1:8080"

max_body_bytes = 1048576      # request-body cap
request_timeout_secs = 30
rate_limit_per_minute = 120   # per key; 0 disables

[[api_keys]]
id = "example-agent"          # audit-log identifier, never the secret
key_hash = "…"                # lowercase hex SHA-256 of the raw key
namespace = "agent"           # "" or "*" = unrestricted
scopes = ["read", "write"]    # any of read | write | delete
```

## Authentication

Every `/v1` request except `GET /v1/health` requires a bearer token:

```
Authorization: Bearer <key>
```

Keys are stored as **SHA-256 hashes**, never in plaintext. The gateway hashes the
presented key and matches it against `key_hash`. Generate a key and its hash:

```bash
KEY=$(openssl rand -hex 32)
echo "raw:  $KEY"                                  # give this to the client
printf '%s' "$KEY" | sha256sum | cut -d' ' -f1     # put this in key_hash
```

### Scopes

Each key grants a set of scopes:

| Scope | Grants |
|-------|--------|
| `read` | query, get, search, recall, relationships |
| `write` | assert, batch, extract, hook ingest |
| `delete` | forget (delete) |

A request needing a scope the key lacks gets `403 Forbidden`.

### Namespace isolation

Each key is bound to a namespace. A key may only act within that namespace or its
children (`"<namespace>:..."`). An empty namespace or `"*"` is unrestricted.

- **Writes** whose target namespace is outside the key's scope are rejected `403`.
- **Reads** are scoped to the key's namespace: if no namespace is given, results
  are restricted to the key's own; a requested namespace outside scope is `403`.
- **Get / relationships / delete by id** on a claim outside the key's namespace
  return `404` (existence is not leaked).

### Rate limiting

Each key has a token bucket of `rate_limit_per_minute` requests, refilled
continuously. Exhaustion returns `429 Too Many Requests`. Set `0` to disable.

## Endpoints

All paths are under `/v1` and speak JSON.

| Method & path | Scope | Purpose |
|---|---|---|
| `GET /v1/health` | — | Gateway liveness + best-effort instance health |
| `POST /v1/claims` | write | Assert one claim |
| `POST /v1/claims/batch` | write | Bulk-learn many claims |
| `GET /v1/claims` | read | Query claims by filter |
| `GET /v1/claims/{id}` | read | Fetch one claim |
| `GET /v1/claims/{id}/relationships` | read | Provenance / contradiction graph |
| `DELETE /v1/claims/{id}` | delete | Forget (real delete) |
| `POST /v1/search` | read | Semantic search |
| `POST /v1/recall` | read | One-call context: merge structured + semantic |
| `POST /v1/extract` | write | Text → claims via the LLM Extractor |
| `POST /v1/hooks/ingest` | write | Claude Code hook event → claims |

### Assert — `POST /v1/claims`

```json
{ "namespace": "agent", "subject": "user:alice", "predicate": "prefers",
  "object": "dark-mode", "confidence": 0.9, "tier": "task" }
```

`confidence` defaults to `0.9`, `tier` to `task`. Response: `{ "id": "<ulid>" }`.

### Batch — `POST /v1/claims/batch`

```json
{ "claims": [
  { "namespace": "agent", "subject": "s", "predicate": "p", "object": "o",
    "confidence": { "lower": 0.7, "upper": 0.9 }, "tier": "task",
    "source_type": "import" }
] }
```

Response: `{ "inserted_count", "duplicate_count", "error_count", "errors" }`.

### Query — `GET /v1/claims`

Query-string filters: `namespace`, `subject`, `predicate`, `object`, `tier`,
`min_confidence`, `source_type`, `limit`. Returns an array of claim objects.

```
GET /v1/claims?namespace=agent&source_type=extraction
```

### Search — `POST /v1/search`

```json
{ "query": "what does alice prefer", "namespace": "agent",
  "limit": 10, "min_similarity": 0.3 }
```

Response: `{ "results": [ { "claim": {…}, "similarity": 0.87 } ] }`. Requires the
instance to have a vector index (a non-`none` embedding backend).

### Recall — `POST /v1/recall`

One call that merges a structured lookup by `subject` with a semantic `search`
over `text`, deduped and ranked (semantic hits first):

```json
{ "subject": "user:alice", "text": "preferences and settings",
  "namespace": "agent", "limit": 10 }
```

At least one of `subject` or `text` is required. Response:
`{ "results": [ { "claim": {…}, "similarity": 0.87 | null } ] }`.

### Extract — `POST /v1/extract`

```json
{ "text": "Alice works at Acme and prefers espresso.",
  "namespace": "agent", "tier": "task", "source_id": "doc-42" }
```

Turns text into claims via the server-side LLM Extractor. Extracted claims carry
`source_type = "extraction"`. Requires `[extraction]` enabled in the instance
config (see below); otherwise the upstream returns `502` with a
"not enabled" message. Response:
`{ "claims_created": [ {…} ], "created_count", "corroborated_count",
"failed_count", "failures" }`.

### Hook ingest — `POST /v1/hooks/ingest`

Accepts a Claude Code hook event (e.g. `PostToolUse`, `UserPromptSubmit`) and
turns it into claims. Default mode is a **deterministic** mapping via `Learn`:

- `PostToolUse` + `Edit`/`Write` → `agent:<session> edited file:<path>`
- `PostToolUse` + `Bash` → `agent:<session> ran_command <cmd>`
- `UserPromptSubmit` → `agent:<session> submitted_prompt <prompt>`

Add `?mode=llm` to route the salient text through the LLM Extractor instead; if
extraction is unavailable it falls back to the deterministic mapping, so ingest
keeps working. Response: `{ "mode", "ingested", … }`. The target namespace comes
from the event's optional `namespace` field, else the key's namespace.

## Claim DTO

```json
{
  "id": "01J...",
  "namespace": "agent",
  "subject": "user:alice",
  "predicate": "prefers",
  "object": "dark-mode",
  "confidence": { "lower": 0.9, "upper": 0.9 },
  "tier": "task",
  "source_type": "assertion",
  "created_at": 1730000000,
  "stale_at": null
}
```

`tier` is one of `ephemeral | task | project | permanent`; `source_type` is one of
`assertion | extraction | inference | import`.

## Errors

Every error response has the shape:

```json
{ "error": "human-readable message", "request_id": "01J..." }
```

The same id is returned in the `x-request-id` header.

| Status | Meaning |
|--------|---------|
| `400` | Bad request (validation) |
| `401` | Missing/invalid API key |
| `403` | Missing scope or namespace out of the key's scope |
| `404` | Claim not found (or outside the key's namespace) |
| `408` | Request timed out |
| `413` | Body exceeds `max_body_bytes` |
| `429` | Rate limit exceeded |
| `502` / `503` | Upstream (Router/instance) error or unavailable |

## Enabling server-side extraction

`/v1/extract` and LLM-mode `/v1/hooks/ingest` need the instance-side Extractor,
which is off by default. Enable it in the instance config and pull the model:

```toml
# config/instance.toml
[extraction]
enabled = true
model = "qwen2.5:7b"
endpoint = "http://localhost:11434"
max_text_length = 50000
```

Deterministic hook ingest (via `Learn`) works whether or not extraction is
enabled.

## Deployment

The gateway serves plain HTTP on localhost. Do not expose it directly. Put one of
these in front for TLS and reach, and keep the gRPC instance bound to `127.0.0.1`:

- **Reverse proxy** (Caddy, nginx, Traefik) terminating TLS.
- **Outbound tunnel** (Cloudflare Tunnel, Tailscale Funnel) so nothing inbound is
  exposed. For "my own machines only", a private overlay (Tailscale without
  Funnel) is stricter than public exposure.

Bind each API key to the narrowest namespace and scope set it needs, keep
`rate_limit_per_minute` conservative, and rotate keys by replacing their hashes.
Every mutation is audit-logged with the key id, namespace, operation, and count.

This is a concrete first step toward Boswell's target security model
([`docs/architecture/10-security.md`](../architecture/10-security.md)) — a single
authenticated, TLS-fronted, namespace-scoped surface — not a replacement for it.
