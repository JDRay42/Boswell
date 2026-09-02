#!/usr/bin/env bash
# Shared helpers for the Boswell <-> Claude Code example hooks.
# Sourced by the sibling hook scripts; not meant to be run directly.

# The boswell CLI binary. Override with BOSWELL_BIN when it is not on PATH, e.g.
#   BOSWELL_BIN="$HOME/Boswell/target/release/boswell"
BOSWELL_BIN="${BOSWELL_BIN:-boswell}"

# The subject id that represents "you" / this agent's principal. Every claim the
# recall hook reads and the capture hook writes hangs off this subject. It must be
# a Boswell entity string: "namespace:value", exactly one colon, lowercase.
BOSWELL_SELF_ID="${BOSWELL_SELF_ID:-person:me}"

# Read the hook event JSON from stdin (may be empty).
read_event() { cat; }

# Capability probes.
have_jq() { command -v jq >/dev/null 2>&1; }
have_boswell() { command -v "$BOSWELL_BIN" >/dev/null 2>&1; }

# Emit an additionalContext payload for a SessionStart / UserPromptSubmit hook.
# Usage: emit_context "<hookEventName>" "<context text>"
# jq handles all string escaping (newlines, quotes) for us.
emit_context() {
  local event="$1" text="$2"
  jq -cn --arg e "$event" --arg c "$text" \
    '{hookSpecificOutput: {hookEventName: $e, additionalContext: $c}}'
}

# Fail OPEN: print nothing, exit 0. A missing or unreachable Boswell must never
# block or break a Claude Code session, so every error path calls this.
bail_open() { exit 0; }
