# Boswell for OpenRig

Files for running Boswell as the shared memory of an OpenRig team. The guide is
[`docs/integrations/openrig.md`](../../docs/integrations/openrig.md).

| File | What it is |
|---|---|
| `runtime/claude-mcp.fragment.json` | Puts `boswell-mcp` in every Claude Code seat |
| `runtime/claude-settings.fragment.json` | Pre-approves that server, so seat startup does not stop at the MCP prompt |
| `runtime/codex-config.fragment.toml` | The same server for Codex seats |
| `plugin/boswell/` | Hooks: a width dossier at session start, a digest of changes at each prompt |
| `agent.yaml.example` | The agent-spec lines that wire the three together |
| `rig-env.example` | The seat environment the hooks read |

The hooks and the bridge are the `boswell-openrig` binary:

```
cargo install --path crates/boswell-openrig
```
