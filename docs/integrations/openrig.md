# Boswell for OpenRig

[OpenRig](https://github.com/mvschwarz/openrig) runs teams of Claude Code and Codex agents in
seats, with a daemon that tracks who owns which work. Boswell can be that team's shared
memory. This guide sets it up.

The two divide the work cleanly. OpenRig answers *what is being worked and who owns it*: its
queue, its slices, its execution view. Boswell answers *what is known and what already exists
to reuse*: claims about components, procedures for using them, open contradictions. A team
with only the first rebuilds things it already has, because nothing tells the agent about to
write a date parser that another seat finished one an hour ago.

The files referred to below are in [`examples/openrig/`](../../examples/openrig/).

## What you get

| Piece | What it does | Needs |
|---|---|---|
| **MCP server in every seat** | Seats get `boswell_dossier`, `boswell_feed` and the rest of Boswell's tools, and what each seat writes is labeled with its session name | `boswell-mcp` |
| **Hooks** | At session start a seat sees the shape of what Boswell holds about the subjects you name; at each prompt it sees what its teammates changed since its last one | `boswell-openrig`, the gateway |
| **Bridge** | While a seat owns a queue item, Boswell shows it as being built; when it finishes, Boswell records who finished it. Contradictions and promotions in Boswell appear in OpenRig's stream | `boswell-openrig`, the gateway |

Each piece works without the others. The MCP server alone is enough for agents that remember
to look.

## The mile-wide view

OpenRig's context packs label material `purpose: width` or `purpose: depth`, and Boswell's
dossier uses the same split. The **width** view lists each `(subject, predicate)` group
Boswell holds about a subject, one line per group:

```
### DateParser
- contested: DateParser is thread safe → yes (2 claims, 2 independent backers)
- thin: DateParser is being built → by impl-1@build (OpenRig queue item q7) (1 claim, 1 independent backer)
- settled: DateParser is located at → src/util/date.rs (3 claims, 2 independent backers)
- Procedures: use-date-parser (effectiveness 0.75)
```

Each line says how far to trust what it reports:

- **contested**: someone recorded a contradiction.
- **thin**: fewer than two independent principals back it.
- **stale**: every claim in the group is past its staleness date.
- **settled**: none of the above.

An agent that wants the detail asks again with `view: depth`, optionally narrowed to one
predicate. It then gets the claims, who wrote each one, and what contradicts them. The
design is [`17-research.md`](../architecture/17-research.md) §5.

## Before you start

- A running Boswell instance and router. Seats reach the instance through `boswell-mcp`.
- The Boswell HTTP gateway ([`http-api.md`](http-api.md)), which the hooks and the bridge
  use.
- Two gateway keys:
  - one shared by the seats, with `read` and `write`;
  - one for the bridge, with `read`, `write` and `delete`, since it forgets announcements
    when work closes.

  Both keys need the namespace the team works in.
- `boswell-mcp` and `boswell-openrig` on the `PATH` the OpenRig daemon launches seats with:

  ```
  cargo install --path crates/boswell-mcp
  cargo install --path crates/boswell-openrig
  ```

- For the hooks, Claude Code 2.1.280 or later. That is the version from which OpenRig loads
  plugin hooks into seats.

## 1. Boswell's tools in every seat

OpenRig merges per-seat *runtime resources* into each seat's configuration. Three fragments
add Boswell:

- `runtime/claude-mcp.fragment.json` is merged into the seat's `.mcp.json` and starts
  `boswell-mcp`.
- `runtime/claude-settings.fragment.json` pre-approves that server. Without it, a new server
  in `.mcp.json` stops seat startup at Claude's approval prompt, which OpenRig reports as
  `mcp_gate`.
- `runtime/codex-config.fragment.toml` does the same for Codex seats.

Declare and select them in the agent spec as `agent.yaml.example` shows. If your spec already
uses OpenRig's shared `claude-default-settings` fragment, check how your OpenRig version merges
the two `enabledMcpjsonServers` arrays. If the later one replaces the earlier, list every
server in one fragment.

**Attribution.** Every agent in the team writes under one principal, `BOSWELL_MCP_PRINCIPAL`
(the fragments use `openrig`). `boswell-mcp` adds the seat's `OPENRIG_SESSION_NAME` as an
agent label, so a claim written by `impl-2@build` is authored `openrig/sub:impl-2@build`.
`BOSWELL_MCP_AGENT` overrides the label. The label shows who wrote what. It never counts as an
independent backer: the whole team is one principal, so its agreement alone reads `thin`
(see [Attribution and independence](#attribution-and-independence)).

`boswell-mcp` talks to the instance directly over gRPC, inside the trust boundary the gateway
draws ([ADR-021](../ADRs/021-gateway-is-the-security-boundary.md)). It is for seats on the same
machine as Boswell. Remote seats should use the gateway.

## 2. The hooks

The plugin in `plugin/boswell/` runs `boswell-openrig` at two points:

- **`SessionStart`** injects a short reminder to check Boswell before building, plus a width
  dossier for each subject in `BOSWELL_SUBJECTS` and for `BOSWELL_TOPIC`. A fresh session then
  starts following the feed from now. A resumed or compacted one keeps its place, so its next
  prompt reports what changed while it was away.
- **`UserPromptSubmit`** injects a digest of what changed in Boswell since the seat's last
  prompt: contradictions first, then demotions and forgets, supersessions, promotions and new
  entries, up to 20 lines. Nothing changed means nothing is injected. The seat's own writes are
  left out. Its teammates' writes are kept, even though they share its key.

A digest looks like this:

```
## Boswell
3 changes in shared memory since you last looked:
- [contradicted] claim: DateParser is thread safe yes (project)
- [superseded] procedure: use-date-parser: parse a timestamp (task)
- [created] claim: Tokenizer is located at src/tok.rs (task) — by impl-1@build
```

**Install.** Copy `plugin/boswell` to `$OPENRIG_HOME/plugins/boswell` and declare it as
`agent.yaml.example` shows. By default OpenRig loads only its own core plugin's hooks into a
seat. Another plugin is projected into the seat but its hooks are not run. OpenRig's
refocus-channel reference documents that a seat-declared `CLAUDE_CODE_PLUGIN_DIRS` is kept,
with OpenRig's core plugin appended. So set it in the seat's environment to the projected
plugin directory (see `rig-env.example`), and check that the hooks fire in one seat before
relying on them. For Codex seats, `hooks/codex.json` is provided, but whether Codex loads
third-party plugin hooks has not been confirmed.

**Configure** with the seat environment in `rig-env.example`. Put no token in the rig spec:
OpenRig stores rig-spec `env:` values in its database and shows them in tmux arguments. Point
`BOSWELL_TOKEN_FILE` at a file only you can read.

**Failure is silent.** A hook sits on the prompt path, so it never blocks a prompt:

- an unconfigured seat gets nothing;
- an unreachable gateway gets nothing after three seconds;
- every error goes to stderr.

Each seat's place in the feed is a cursor file under `$OPENRIG_HOME/boswell/cursors/`. Deleting
the file makes the seat start following from now.

## 3. The bridge

```
BOSWELL_URL=http://127.0.0.1:8081 BOSWELL_TOKEN_FILE=~/.openrig/boswell/bridge-token \
BOSWELL_NAMESPACE=team boswell-openrig bridge
```

Run one bridge per OpenRig daemon, under whatever keeps your other services up (launchd,
systemd, a tmux pane). It needs no inbound port.

**Inbound**, from OpenRig's `/api/events`:

| OpenRig event | Boswell write |
|---|---|
| `queue.claimed`, or `queue.updated` to `in-progress` | Assert `(<item summary>, "is being built", "by <seat> (OpenRig queue item <id>)")`, labeled with the seat |
| `queue.updated` to `done` | Forget the announcement; assert `(<item summary>, "was completed by", <seat>)` |
| `queue.updated` to `pending`, `failed`, `denied`, `canceled` or `handed-off`; `queue.unclaimed`; `queue.handed_off` | Forget the announcement |
| `queue.updated` to `blocked` | Nothing: the item is still owned |

This turns the queue into the announcements the change-feed design calls for
([`18-change-feed.md`](../architecture/18-change-feed.md) §6.1). An agent about to start
something sees in a dossier that a teammate already has it, and its next prompt digest
carries the announcement. An item with no summary is announced as
`OpenRig queue item <id>`.

**Outbound**, from Boswell's feed to OpenRig's `/api/stream/emit`:

- **What goes out.** By default, contradictions, promotions and supersessions;
  `BOSWELL_BRIDGE_CHANGES` changes the list.
- **How it appears.** Each change is posted as a stream item from `boswell@bridge` with
  `hintType: memory`. Contradictions are `urgent` and everything else is `routine`.
- **Idempotent ids.** The stream item id comes from the feed cursor, so a repost after a
  crash is the same item.
- **No echo.** The bridge reads the feed with `exclude_self`, so its own announcements are not
  sent back to the rig. This is why the bridge needs its own key: on the seats' key, it would
  also drop everything the seats wrote.

**State** lives in `$OPENRIG_HOME/boswell/bridge.json`:

- the last OpenRig event it handled, sent as `Last-Event-ID` on reconnect, with `replay=0` so
  the daemon never replays its whole log;
- its Boswell cursor;
- which claim announces which queue item.

An event Boswell refuses is not passed. The bridge reconnects and retries it, backing off to
30 seconds.

## Recording what gets built

Nothing above helps if finished work never reaches Boswell. Tell your agents, in the agent
spec's instructions or the rig's `CULTURE.md`, to record a component when they finish it:

- claims for what it is and where it lives: `(<name>, "is located at", <path>)`, plus one
  `(<name>, "provides", <capability>)` per capability, so a dossier on the capability finds
  it;
- a procedure for using it, placed under the goal it serves.

The guidance in [`18-change-feed.md`](../architecture/18-change-feed.md) §6.2 applies. Name
things the way the next agent will search for them: a dossier anchors on exact subjects, and
"DateParser" and "date parser" are different subjects.

## Attribution and independence

Boswell counts how many independent principals back a claim, and an agent label never makes
another one ([ADR-023](../ADRs/023-questions-and-dossiers.md)). A rig is one principal however
many seats it runs, so a claim only the rig's seats agree on reads `thin`. That is deliberate.
Seats share a prompt lineage, a model and usually their sources, and letting them corroborate
each other would let a team promote its own guesses. A claim becomes `settled` when a second
principal backs it: another person's agents, a reviewer, or an imported source.

## Environment reference

| Variable | Read by | Meaning | Default |
|---|---|---|---|
| `BOSWELL_URL` | hooks, bridge | Gateway base URL, without `/v1` | none; unset disables hooks |
| `BOSWELL_TOKEN` / `BOSWELL_TOKEN_FILE` | hooks, bridge | Gateway key, or a file holding it | none |
| `BOSWELL_NAMESPACE` | hooks, bridge | Namespace to read and write | the key's own; the bridge writes to `openrig` |
| `BOSWELL_SUBJECTS` | `SessionStart` | Comma-separated subjects for width dossiers | none |
| `BOSWELL_TOPIC` | `SessionStart` | A semantic anchor (needs vector search) | none |
| `BOSWELL_OPENRIG_STATE_DIR` | hooks, bridge | Where cursors and bridge state live | `$OPENRIG_HOME/boswell` |
| `OPENRIG_URL` | bridge | The OpenRig daemon | `http://127.0.0.1:7433` |
| `BOSWELL_BRIDGE_CHANGES` | bridge | Changes posted to OpenRig | `contradicted,climbed,superseded` |
| `BOSWELL_BRIDGE_POLL_SECS` | bridge | Seconds between Boswell feed polls | `15` |
| `BOSWELL_ROUTER` | `boswell-mcp` | Boswell router URL | `http://localhost:8080` |
| `BOSWELL_MCP_PRINCIPAL` | `boswell-mcp` | The principal seats write under | `mcp` |
| `BOSWELL_MCP_AGENT` | `boswell-mcp` | The agent label | `OPENRIG_SESSION_NAME` |

## Limits

- **The feed is polled.** The hooks read it at each prompt and the bridge every 15 seconds.
  Server-sent events are designed but not built.
- **Questions are not built.** Dossiers report them `unavailable`, and recording what a team
  could not find waits on the `Question` store
  ([ADR-023](../ADRs/023-questions-and-dossiers.md)).
- **Subjects are free strings.** The bridge announces under a queue item's summary, which
  rarely matches the name the finished component is recorded under. The announcement says
  *someone is on this*. The completed component's own claims say *here it is*.
- **Intent matching is substring.** Procedures and goals join a dossier by substring until
  semantic intent matching ships. The dossier's `coverage` says which was used.
- **Codex hooks are unconfirmed.** The Codex plugin manifest and hooks are provided; the
  plugin-hook loading they rely on has been confirmed for Claude Code only.

## Troubleshooting

- **A seat gets no Boswell context.**
  1. Run `boswell-openrig hook session-start </dev/null` in the seat's environment. Errors go
     to stderr.
  2. Check `BOSWELL_URL` and the token file.
  3. Check that the plugin's hooks are loaded (`CLAUDE_CODE_PLUGIN_DIRS`).
- **Seat startup stops at `mcp_gate`.** The settings fragment did not take effect, so
  `boswell` is not in `enabledMcpjsonServers`.
- **Every claim in a dossier reads `thin`.** This is expected while only the rig writes
  (see [Attribution and independence](#attribution-and-independence)).
- **The bridge logs `403`.** Its key lacks `delete`, or the namespace.
- **The bridge logs `cursor_gone`.** Its Boswell cursor fell behind `[feed] retention_days`.
  It resumes from the head on its own. Anything it missed is in Boswell, not in OpenRig's
  stream.
