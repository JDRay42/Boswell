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

## The `PostToolUse` HTTP hook in the example

`settings.example.json` also shows a native `type: "http"` hook posting to
`https://boswell.example.com/hooks/ingest`. That endpoint is a **proposed design, not
yet implemented** — Boswell's Router today exposes only `/session/establish` and
`/health`. It is included so the config shows both integration styles side by side; see
the [integration guide](../../docs/integrations/claude-code-hooks.md) for the endpoint
and security design. Remove that block until the endpoint exists.
