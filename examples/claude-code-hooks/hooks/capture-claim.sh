#!/usr/bin/env bash
# UserPromptSubmit hook — capture a lightweight, deterministic claim into Boswell.
#
# This example records the project you are actively working in, derived from the
# session's working directory:
#     (BOSWELL_SELF_ID) rel:working-on project:<dir>   [tier: task]
#
# It is deliberately deterministic. Turning a full prompt or transcript into rich,
# high-quality claims is the job of Boswell's Extractor (LLM-backed), not a shell
# hook — see docs/integrations/claude-code-hooks.md for how the HTTP-ingest path
# would do that server-side.
#
# Requires: a running boswell-server + boswell-router, plus `jq`.
# Fails OPEN: never blocks the prompt; the write is best-effort.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/common.sh
. "$DIR/lib/common.sh"

event="$(read_event)"
have_jq || bail_open
have_boswell || bail_open

cwd="$(printf '%s' "$event" | jq -r '.cwd // empty' 2>/dev/null)"
[ -n "$cwd" ] || cwd="$PWD"

# Normalize the directory name into a Boswell entity value: lowercase, separators
# to hyphens, drop anything that is not [a-z0-9-].
project="$(basename "$cwd" | tr '[:upper:]' '[:lower:]' | tr ' _' '--' | tr -cd 'a-z0-9-')"
[ -n "$project" ] || bail_open

claims="$(jq -cn --arg s "$BOSWELL_SELF_ID" --arg o "project:$project" '
  [ { subject: $s, predicate: "rel:working-on", object: $o,
      confidence: { lower: 0.6, upper: 0.9 }, tier: "task" } ]')"

# Gate the load on an offline schema validation of the exact payload, mirroring
# docs/importing-personal-memory.md ("validate && learn").
printf '%s' "$claims" | "$BOSWELL_BIN" validate --stdin >/dev/null 2>&1 || bail_open
printf '%s' "$claims" | "$BOSWELL_BIN" learn --stdin >/dev/null 2>&1 || bail_open

exit 0
