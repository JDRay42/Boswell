# Integrating Claude Code hooks with Boswell

[Claude Code hooks](https://code.claude.com/docs/en/hooks) run user-defined actions
at points in an agent's lifecycle. They are a natural way to give a coding agent a
persistent memory: recall what Boswell knows when a session starts, and capture new
knowledge as the session runs.

This guide covers two integration transports and, because one of them raises the
question, how to serve Boswell **securely while making it publicly reachable**.

Runnable examples for the local path live in
[`examples/claude-code-hooks/`](../../examples/claude-code-hooks/).

## Which hook events matter for a memory system

| Event | Memory role |
|-------|-------------|
| `SessionStart` | **Recall.** Query Boswell and inject claims as `additionalContext` so the agent starts informed. |
| `UserPromptSubmit` | **Recall (scoped)** on the prompt, and/or **capture** of what the user is asking about. |
| `Stop` / `SubagentStop` | **Capture.** Persist what was learned or done during the turn. |
| `PostToolUse` | **Capture (granular).** Record edits, commands, or outcomes as they happen. |

## Two transports

### 1. Command hooks → the `boswell` CLI (works today)

A `type: "command"` hook is a shell script. It receives the hook event JSON on stdin
and shells out to the `boswell` CLI, which already handles the Router session +
gRPC call internally. This is the simplest path and needs **no network exposure** —
the agent and Boswell run on the same machine over `localhost`.

The examples directory implements this end to end:

- `session-start-recall.sh` runs `boswell query --format json` and returns
  `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"…"}}`.
- `capture-claim.sh` builds a claim array and pipes it through
  `boswell validate --stdin && boswell learn --stdin` (the same "validate then learn"
  gate as [Importing Personal Memory](../importing-personal-memory.md)).

Both **fail open**: if Boswell is unreachable they print nothing and exit `0`, so a
down memory service never blocks the session.

### 2. Native HTTP hooks → an HTTP endpoint

A `type: "http"` hook makes Claude Code POST the event JSON directly to a URL, with
header env-var interpolation and an `allowedEnvVars` allowlist so secrets stay out of
`settings.json`:

```json
{
  "type": "http",
  "url": "https://boswell.example.com/v1/hooks/ingest",
  "headers": { "Authorization": "Bearer $BOSWELL_TOKEN" },
  "allowedEnvVars": ["BOSWELL_TOKEN"]
}
```

Two things follow from this:

1. **The payload is Claude Code *event* JSON** (`hook_event_name`, `tool_name`,
   `tool_input`, `prompt`, `cwd`, …) — **not** Boswell claim JSON, and it does not go
   through the `/session/establish` → gRPC flow directly. It needs a Boswell endpoint
   that accepts that shape and turns it into claims. **That endpoint now exists**:
   `POST /v1/hooks/ingest` on the [`boswell-gateway`](http-api.md), which maps the
   event to claims (deterministically, or via the LLM Extractor with `?mode=llm`).
2. **The URL is reached over the network**, which is where "secure + publicly
   accessible" comes in.

The `boswell-gateway` implements the authenticated, namespace-scoped endpoint this
guide describes. See the [HTTP API guide](http-api.md) for the full surface; the rest
of this section covers how to deploy it securely.

## Serving Boswell: local by default, public by opt-in

### Local (default, most secure)

Run the hook and Boswell on the same machine. Command hooks talk to `localhost`;
an HTTP hook targets the gateway at `http://127.0.0.1:8081/v1/hooks/ingest`. Nothing
listens on a public interface, the gRPC instance stays bound to `127.0.0.1:50051`, and
there is no attack surface beyond the machine itself. **Prefer this whenever the agent runs where Boswell
runs.** You only need the public path when a *remote* agent — Claude Code on the web,
a cloud session, or a teammate's machine — must reach a hosted Boswell.

### Public (opt-in) — the design

The goal: expose exactly one hardened HTTP entrypoint, authenticated, over TLS, without
opening the instance's gRPC port to the world.

**1. Keep gRPC private.** `boswell-server` (:50051) stays bound to `127.0.0.1`. Never
expose the instance directly; the gRPC `auth_token` check is not a public-facing
authorization boundary.

**2. One public entrypoint — `POST /v1/hooks/ingest` on the gateway.** The
`boswell-gateway` is a slim service in front of the private gRPC instance that:

- accepts Claude Code hook event JSON,
- maps it to claims — deterministically (e.g. `PostToolUse`/`Edit` →
  `agent:<session> edited file:<path>`) via `Learn`, or with `?mode=llm` hands the
  salient text to Boswell's LLM Extractor for richer claims,
- validates and asserts them through the existing store path.

**3. Transport security — don't open an inbound port.**

- **Reverse proxy** (Caddy / nginx / Traefik) terminating TLS in front of the Router,
  or
- **Outbound tunnel** — [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/)
  or [Tailscale Funnel](https://tailscale.com/kb/1223/funnel) — so the host makes an
  outbound connection and nothing inbound is exposed. If "public" really means "my own
  machines and agents," a private overlay (Tailscale, no Funnel) is stricter than true
  public exposure; use Cloudflare Tunnel + [Access](https://developers.cloudflare.com/cloudflare-one/policies/access/)
  when it must be internet-reachable.

**4. Authentication.** A bearer token in the `Authorization` header, supplied by the
hook via `headers` + `allowedEnvVars` so it never sits in `settings.json`. Prefer a
**dedicated, revocable, namespace-scoped API key** over reusing the Router's JWT: the
JWT is a short-lived session artifact, whereas a hook needs a long-lived credential you
can rotate and revoke independently.

**5. Authorization / isolation.** Bind each key to a **namespace** and a **maximum
tier**, so a compromised hook key can only write claims into its own namespace and
cannot, say, forge `permanent`-tier claims in someone else's namespace.

**6. Hardening.** Payload-size cap, per-key rate limiting, a short request timeout,
reject-on-schema-mismatch, and an audit log of every ingest (key id, namespace, claim
count, decision). Optionally add HMAC request signing for integrity on top of TLS.

**7. Secret hygiene.** Only pass secrets through headers + `allowedEnvVars`. Hook
stdout/stderr can surface in Claude Code debug logs and transcripts, so never echo a
token; keep it in the header path.

**8. Alignment with Boswell's security roadmap.** Boswell's target model
([`docs/architecture/10-security.md`](../architecture/10-security.md)) is mTLS
everywhere with per-instance signed tokens. That is not implemented yet. The ingest-auth
design above is a concrete, shippable first step toward it — a single authenticated,
TLS-fronted, namespace-scoped surface — rather than a replacement for it.

### Threat model at a glance

| Posture | Exposure | AuthN | Residual risk |
|---------|----------|-------|---------------|
| **Localhost** | None (127.0.0.1) | OS/user boundary | Only local processes; simplest and safest. |
| **Private overlay** (Tailscale, no Funnel) | Reachable by your tailnet devices | Overlay identity + bearer token | Compromise of an enrolled device or key. |
| **Public + auth** (tunnel/proxy + API key) | Internet-reachable | Bearer API key, namespace-scoped, rate-limited | Key leakage; mitigated by scope, rotation, revocation, audit. |

## Status

- **Command-hook path:** implemented and runnable — see
  [`examples/claude-code-hooks/`](../../examples/claude-code-hooks/).
- **HTTP-hook path (`/v1/hooks/ingest` + auth + deployment):** implemented by the
  [`boswell-gateway`](http-api.md). Point the `type: "http"` hook in
  `settings.example.json` at your gateway (default
  `http://127.0.0.1:8081/v1/hooks/ingest`) with a bearer key. Deployment (TLS via
  proxy/tunnel) is still yours to run per the guidance above.
