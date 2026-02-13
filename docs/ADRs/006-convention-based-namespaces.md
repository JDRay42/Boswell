# ADR-006: Convention-Based Hierarchical Namespaces

## Status

Accepted

## Context

Claims need lateral isolation (development vs. cooking vs. personal) in addition to tiered isolation (ephemeral vs. persistent). Three approaches were evaluated:

1. **Flat string**: Simple, fast, but no inherent hierarchy. Querying "everything under project X" requires exact knowledge of all child namespaces.
2. **Tree structure (adjacency table)**: Full hierarchical model with parent-child relationships. Supports recursive queries natively but adds schema complexity, joins, and potential for unbounded depth.
3. **Convention-based hierarchy**: Flat string with a path convention (`segment/segment/segment`), queryable via prefix matching. Hierarchy is implicit in the string, not enforced by schema.

## Decision

**Convention-based hierarchy with slash-count enforcement.** Namespaces are plain strings stored in a single column. The hierarchy is expressed via slash-delimited segments (`development/boswell/claim-store`). Maximum depth is enforced by counting slash characters at write time — a deterministic validation function, not an LLM call.

## Consequences

- **Simple storage**: single string column, no tree tables, no adjacency lists.
- **Fast queries**: prefix matching (`starts_with`) is indexable and efficient. No recursive joins.
- **Three query modes**: exact match, recursive (prefix match), and depth-limited (prefix match + slash count filter).
- **Guardrailed depth**: slash-count validation prevents runaway nesting. Recommended limit: 4-5 slashes. Configurable per instance, potentially per tier.
- **Namespace discovery**: a `DISTINCT` query on the namespace column with prefix filtering lets agents orient themselves.
- **No automatic scope inheritance**: querying a parent namespace does not automatically include children. Recursive queries are opt-in via the `/*` suffix. This keeps scopes from leaking unintentionally.
