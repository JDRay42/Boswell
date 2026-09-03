#!/usr/bin/env bash
# SessionStart hook — recall long-term memory from Boswell.
#
# Queries Boswell for the claims stored about this agent's principal
# (BOSWELL_SELF_ID) and injects them into the session as `additionalContext`, so
# Claude starts each session already knowing what Boswell remembers about you.
#
# Wire-up: SessionStart -> type "command" -> this script (see settings.example.json).
# Requires: a running boswell-server + boswell-router, plus `jq`.
# Fails OPEN: if Boswell is unreachable or empty, the session starts normally with
# nothing injected.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
. "$DIR/lib/common.sh"

_=$(read_event) # drain the SessionStart event JSON on stdin; no fields needed here

have_jq || bail_open
have_boswell || bail_open

# `query --format json` emits an array of
#   {id, namespace, subject, predicate, object, confidence:{lower,upper}, tier, ...}
claims_json="$("$BOSWELL_BIN" query --subject "$BOSWELL_SELF_ID" --format json 2>/dev/null)" || bail_open
[ -n "$claims_json" ] || bail_open

count="$(printf '%s' "$claims_json" | jq 'length' 2>/dev/null)" || bail_open
[ "${count:-0}" -gt 0 ] || bail_open

context="$(printf '%s' "$claims_json" | jq -r '
  "Recalled from Boswell — claims about you (subject · predicate · object [confidence], tier):",
  ( sort_by(.tier)[] |
      "- \(.subject) · \(.predicate) · \(.object) [\(.confidence.lower)–\(.confidence.upper)] (\(.tier))" )
' 2>/dev/null)" || bail_open
[ -n "$context" ] || bail_open

emit_context "SessionStart" "$context"
