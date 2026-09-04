# Claude Code hooks for Boswell

Example [Claude Code hooks](https://code.claude.com/docs/en/hooks) that wire an
agent's session lifecycle into Boswell's memory:

- **Recall** — a `SessionStart` hook reads the claims Boswell stores about you and
  injects them into the session as context.
- **Capture** — a `UserPromptSubmit` hook writes a lightweight claim back to Boswell.

Both run locally against your own Boswell instance over `localhost` — no network
exposure. For the remote/public story (and the native HTTP-hook path), see
[`docs/integrations/claude-code-hooks.md`](../../docs/integrations/claude-code-hooks.md).

> These examples are **not** installed as a live `.claude/settings.json`, so opening
> this repo in Claude Code does not silently activate any hooks. You opt in by copying
> the snippet below into your own settings.

## Files

| File | Purpose |
|------|---------|
| `hooks/session-start-recall.sh` | `SessionStart`: query Boswell, inject recalled claims as `additionalContext`. |
| `hooks/capture-claim.sh` | `UserPromptSubmit`: record `(you) rel:working-on project:<dir>` as a claim. |
| `hooks/lib/common.sh` | Shared helpers (stdin, capability probes, `additionalContext` emit, fail-open). |
| `settings.example.json` | Ready-to-paste `hooks` block wiring the two scripts, plus a native HTTP-hook example. |

## Prerequisites

- `jq` on your PATH.
- A running Boswell stack the `boswell` CLI can reach:

  ```bash
  cargo build --release
  ./target/release/boswell-server --config config/instance.toml   # terminal 1
  ./target/release/boswell-router --config config/router.toml     # terminal 2
  ```

  For offline use with no Ollama, set `backend = "mock"` under `[embedding]` in
  `config/instance.toml`.
- Either `boswell` on your PATH, or point the hooks at the built binary:

  ```bash
  export BOSWELL_BIN="$PWD/target/release/boswell"
  ```

## Install

1. Make the scripts executable:

   ```bash
   chmod +x examples/claude-code-hooks/hooks/*.sh
   ```

2. Merge the `hooks` block from `settings.example.json` into your Claude Code
   settings — project (`.claude/settings.json`) or user (`~/.claude/settings.json`).
   `$CLAUDE_PROJECT_DIR` resolves to your project root.

3. (Optional) Seed some memory first so recall has something to show — see
   [Importing Personal Memory](../../docs/importing-personal-memory.md).

## Configuration (env vars)

| Var | Default | Meaning |
|-----|---------|---------|
| `BOSWELL_SELF_ID` | `person:me` | The subject id claims are read from / written to. Use the same id you seeded memory under. |
| `BOSWELL_BIN` | `boswell` | Path to the CLI if it is not on your PATH. |

## Test without Claude Code

The hooks just read a JSON event on stdin and print JSON (or nothing) on stdout:

```bash
# Recall — prints an additionalContext payload if Boswell has claims for BOSWELL_SELF_ID
echo '{"hook_event_name":"SessionStart","source":"startup"}' \
  | examples/claude-code-hooks/hooks/session-start-recall.sh

# Capture — writes a claim, then confirm it landed
echo '{"hook_event_name":"UserPromptSubmit","cwd":"'"$PWD"'","prompt":"hi"}' \
  | examples/claude-code-hooks/hooks/capture-claim.sh
boswell query --subject "${BOSWELL_SELF_ID:-person:me}" --predicate rel:working-on
```

With Boswell stopped, both scripts print nothing and exit `0` — they **fail open** so
memory being down never blocks your session.

## Procedure effectiveness capture (procedural memory, Phase 5)

Boswell's procedural memory learns which procedures actually work by tracking the
outcome of each execution (design §3.3). The mechanism is an **execution receipt**:

1. When an agent retrieves a procedure to run, Boswell issues a *pending* receipt
   (`SqliteStore::issue_receipt`) carrying `procedure_id`, `version`, the principal,
   task/session ids, and an `expires_at` deadline.
2. When the agent finishes, a capture hook reports the outcome
   (`success` / `failure{failure_mode}` / `abandoned`) against that receipt, which
   Boswell applies to the procedure's effectiveness as a gatekept, provenance-stamped
   write (`SqliteStore::report_receipt`).
3. **Silence is not success.** A receipt that expires unreported is swept to
   `expired` by the Janitor (`SqliteStore::expire_receipts`, wired into the sweep
   loop) and counts as an `unknown` — mildly negative for the procedure's
   effectiveness — so an agent cannot game the stats by running a procedure and
   staying quiet on failure.

The natural hook points are:

| Hook | Reports |
|------|---------|
| `SubagentStop` | the outcome of a subagent that executed a procedure |
| `Stop` | the outcome of the main agent's procedure run |
| `PostToolUse` | fine-grained step outcomes during execution |

The store-side capture engine (receipts, report application, expiry) ships in this
phase and is covered by unit tests. The **transport** a hook calls to reach
`report_receipt` — a `boswell procedure report <receipt-id> --outcome …` CLI
subcommand or a gateway endpoint — lands with the other procedure endpoints (a
later slice); until then these hooks are documented here as the intended shape, not
installed scripts.

## The `PostToolUse` HTTP hook in the example

`settings.example.json` also shows a native `type: "http"` hook posting to
`https://boswell.example.com/v1/hooks/ingest`. That endpoint is **implemented** by the
[`boswell-gateway`](../../docs/integrations/http-api.md): it accepts Claude Code hook
event JSON and turns it into claims (deterministically, or via the LLM Extractor with
`?mode=llm`). Point the URL at your gateway (default
`http://127.0.0.1:8081/v1/hooks/ingest`) and supply a bearer API key. See the
[HTTP API guide](../../docs/integrations/http-api.md) for auth and deployment.
