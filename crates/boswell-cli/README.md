# Boswell CLI

Command-line interface for the Boswell cognitive memory system.

## Installation

```bash
cargo install --path crates/boswell-cli
```

This installs the `boswell` binary to your Cargo bin directory.

## Quick Start

```bash
# Connect to a Boswell router
boswell connect --host localhost --port 8080

# Assert a new claim
boswell assert user:alice likes:coffee beverage:espresso --confidence 0.8 0.9

# Query claims
boswell query --subject alice

# Enter interactive mode
boswell repl
```

## Commands

### connect

Connect to a Boswell router and save the connection as the active profile.

```bash
boswell connect [OPTIONS]

Options:
  -H, --host <HOST>              Router host [default: localhost]
  -p, --port <PORT>              Router port [default: 8080]
      --instance-id <ID>         Optional instance ID
      --profile-name <NAME>      Save connection as named profile
```

**Examples:**

```bash
# Connect to local router
boswell connect

# Connect to remote router
boswell connect --host remote.example.com --port 9000

# Save as named profile
boswell connect --host prod.example.com --profile-name production
```

### assert

Assert a new claim into Boswell's cognitive memory.

```bash
boswell assert <SUBJECT> <PREDICATE> <OBJECT> [OPTIONS]

Arguments:
  <SUBJECT>      Subject entity (format: namespace:value)
  <PREDICATE>    Predicate entity (format: namespace:value)
  <OBJECT>       Object entity (format: namespace:value)

Options:
  -c, --confidence <LOW> <HIGH>  Confidence interval [default: 0.7 0.9]
  -t, --tier <TIER>              Storage tier [default: task]
                                 [possible values: ephemeral, task, project, permanent]
```

**Storage Tiers:**

- **ephemeral**: Short-lived claims (deleted after session)
- **task**: Task-scoped claims (deleted after task completion)
- **project**: Project-scoped claims (persist across tasks)
- **permanent**: Permanent claims (never auto-deleted)

**Examples:**

```bash
# Simple assertion
boswell assert user:alice likes:coffee beverage:espresso

# With confidence interval
boswell assert user:bob knows:python lang:python --confidence 0.9 0.95

# With storage tier
boswell assert project:boswell uses:rust lang:rust --tier permanent
```

### query

Query claims from Boswell's memory using filters.

```bash
boswell query [OPTIONS]

Options:
  -n, --namespace <NAMESPACE>    Filter by namespace
  -s, --subject <SUBJECT>        Filter by subject
  -p, --predicate <PREDICATE>    Filter by predicate
  -o, --object <OBJECT>          Filter by object
  -t, --tier <TIER>              Filter by storage tier
  -l, --limit <LIMIT>            Maximum results [default: 100]
```

**Examples:**

```bash
# Query all claims about alice
boswell query --subject alice

# Query all "likes" relationships
boswell query --predicate likes

# Query with multiple filters
boswell query --subject bob --namespace user

# Limit results
boswell query --limit 10
```

### learn

Batch assert multiple claims from a JSON file.

```bash
boswell learn <FILE>

Arguments:
  <FILE>    Path to JSON file containing claims
```

**JSON Format:**

```json
[
  {
    "subject": "user:alice",
    "predicate": "likes:coffee",
    "object": "beverage:espresso",
    "confidence": [0.8, 0.9],
    "tier": "task"
  },
  {
    "subject": "user:bob",
    "predicate": "knows:python",
    "object": "lang:python"
  }
]
```

**Examples:**

```bash
# Import claims from file
boswell learn claims.json

# With different output format
boswell learn --format json claims.json
```

### forget

Delete claims by ID with optional confirmation.

```bash
boswell forget <IDS>... [OPTIONS]

Arguments:
  <IDS>...    Claim IDs to delete (or use --file)

Options:
  -f, --file <FILE>    Read claim IDs from file (one per line)
  -y, --yes            Skip confirmation prompt
```

**Examples:**

```bash
# Delete single claim
boswell forget 01ARZ3NDEKTSV4RRFFQ69G5FAV

# Delete multiple claims
boswell forget 01ARZ3NDEKTSV4RRFFQ69G5FAV 01ARZ3NDEKTSV4RRFFQ69G5FAW

# Delete from file without confirmation
boswell forget --file to-delete.txt --yes
```

### search

Semantic search for claims using vector similarity.

```bash
boswell search <QUERY> [OPTIONS]

Arguments:
  <QUERY>    Search query text

Options:
  -l, --limit <LIMIT>    Maximum results [default: 10]
```

**Note:** This feature requires the Boswell router to expose HNSW vector search capabilities.

**Examples:**

```bash
# Search for claims about programming
boswell search "programming languages"

# Limit search results
boswell search "coffee preferences" --limit 5
```

### profile

Manage configuration profiles for different Boswell instances.

```bash
boswell profile <COMMAND>

Commands:
  list      List all profiles
  show      Show profile details
  switch    Switch active profile
  set       Create or update profile settings
  delete    Delete a profile
```

**Examples:**

```bash
# List all profiles
boswell profile list

# Show active profile
boswell profile show

# Show specific profile
boswell profile show production

# Switch to different profile
boswell profile switch production

# Update profile setting
boswell profile set production host remote.example.com

# Delete profile
boswell profile delete old-dev
```

### repl

Enter interactive REPL (Read-Eval-Print Loop) mode.

```bash
boswell repl [OPTIONS]

Options:
  (inherits global options)
```

**REPL Commands:**

All standard commands are available in REPL mode without the `boswell` prefix:

```
boswell> connect --host localhost
boswell> assert user:alice likes:coffee beverage:espresso
boswell> query --subject alice
boswell> exit
```

**REPL Features:**

- Command history (saved to `~/.boswell/history.txt`)
- Line editing with Ctrl+Left/Right (word navigation)
- Ctrl+C to cancel current line
- Ctrl+D or `exit` to quit

## Global Options

These options apply to all commands:

```bash
-f, --format <FORMAT>
    Output format [possible values: table, json, quiet] [default: table]

--no-color
    Disable colored output

-c, --config <CONFIG>
    Configuration file path [default: ~/.boswell/config.toml]

-p, --profile <PROFILE>
    Profile to use (overrides active profile)
```

## Output Formats

### Table (default)

Human-readable table format with colored output:

```
┌────────────────────────────────┬───────────┬───────────┬───────────┬────────────────┬──────┐
│ ID                             │ Subject   │ Predicate │ Object    │ Confidence     │ Tier │
├────────────────────────────────┼───────────┼───────────┼───────────┼────────────────┼──────┤
│ 01ARZ3NDEKTSV4RRFFQ69G5FAV     │ user:alice│ likes:...  │ bevera... │ [0.80, 0.90]   │ task │
└────────────────────────────────┴───────────┴───────────┴───────────┴────────────────┴──────┘
```

### JSON

Machine-readable JSON format:

```json
[
  {
    "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    "namespace": "default",
    "subject": "user:alice",
    "predicate": "likes:coffee",
    "object": "beverage:espresso",
    "confidence": [0.8, 0.9],
    "tier": "task"
  }
]
```

### Quiet

Minimal output showing only claim IDs (one per line):

```
01ARZ3NDEKTSV4RRFFQ69G5FAV
01ARZ3NDEKTSV4RRFFQ69G5FAW
```

## Configuration

Configuration is stored in `~/.boswell/config.toml`:

```toml
active_profile = "default"

[profiles.default]
host = "localhost"
port = 8080
output_format = "table"
color_enabled = true

[profiles.production]
host = "prod.example.com"
port = 9000
instance_id = "prod-01"
output_format = "json"
color_enabled = false
```

**Profile Settings:**

- `host`: Router hostname (required)
- `port`: Router port (required)
- `instance_id`: Specific instance ID (optional)
- `output_format`: Default output format (optional)
- `color_enabled`: Enable colored output (optional)

## Examples

### Basic Workflow

```bash
# 1. Connect to router
boswell connect

# 2. Assert some claims
boswell assert user:alice likes:coffee beverage:espresso
boswell assert user:alice knows:rust lang:rust
boswell assert user:bob likes:tea beverage:green-tea

# 3. Query what alice likes
boswell query --subject alice --predicate likes

# 4. Forget a claim
boswell query --subject bob --format quiet > bob-claims.txt
boswell forget --file bob-claims.txt --yes
```

### Multi-Environment Setup

```bash
# Set up dev environment
boswell connect --host localhost --port 8080 --profile-name dev

# Set up production environment
boswell connect --host prod.example.com --port 9000 --profile-name prod

# Switch between environments
boswell profile switch dev
boswell assert user:test-user likes:testing test:data

boswell profile switch prod
boswell query --subject production-user
```

### Batch Operations

```bash
# Create claims file
cat > claims.json <<EOF
[
  {"subject": "user:alice", "predicate": "likes:coffee", "object": "beverage:espresso"},
  {"subject": "user:bob", "predicate": "likes:tea", "object": "beverage:green-tea"},
  {"subject": "user:carol", "predicate": "likes:water", "object": "beverage:sparkling"}
]
EOF

# Import all claims
boswell learn claims.json

# Query and export as JSON
boswell query --namespace user --format json > user-claims.json

# Delete all user claims
boswell query --namespace user --format quiet | xargs boswell forget --yes
```

## Error Handling

The CLI provides clear error messages for common issues:

```bash
# Connection error
$ boswell assert user:alice likes:coffee beverage:espresso
Error: Failed to connect to router: Connection refused

# Invalid claim ID
$ boswell forget invalid-id
Error: Invalid claim ID format: invalid-id

# Missing profile
$ boswell --profile nonexistent query
Error: Profile 'nonexistent' not found
```

## Exit Codes

- `0`: Success
- `1`: Error (with message to stderr)

## Development

### Running Tests

```bash
cargo test -p boswell-cli
```

### Building from Source

```bash
cargo build --release -p boswell-cli
```

The binary will be available at `target/release/boswell`.

## See Also

- [Boswell Architecture Documentation](../../docs/architecture/)
- [Boswell SDK](../boswell-sdk/README.md)
- [Boswell MCP Server](../boswell-mcp/README.md)

## License

See the root [LICENSE](../../LICENSE) file for details.
