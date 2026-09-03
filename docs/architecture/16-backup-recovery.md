# Backup & Recovery

**Status:** Design (not yet implemented). A proposal for a durability strategy, not a
description of shipped code.

## 1. Motivation

Boswell is a **durable memory substrate**, run local-first on the operator's own hardware.
Its state is the point of the system, so it needs a recovery story for the ordinary risks of
any local datastore — disk loss, a bad migration, an interrupted write — and for one
Boswell-specific risk: a low-trust or runaway agent **poisoning** the store faster than the
gatekeeping can contain it.

Recovery is the backstop that lets Boswell take a pragmatic stance on adversarial writes
(see the trust posture in `15-procedural-memory.md`): the defenses raise the cost of casual
poisoning, and recovery bounds the damage when something gets through. This document covers
what to back up, how to do it consistently, and how to restore.

## 2. What must be backed up — the database is the source of truth

For the default SQLite adapter, the essential artifact is a **single file: the SQLite
database** — claims, relationships, provenance, the confidence cache, and the embedding
vectors themselves (stored in the `embedding_vector` column; `boswell-store`, `schema.sql`).

The **HNSW vector index** (a separate, memory-mapped file; `boswell-store/src/vector_index.rs`)
is a **derived, rebuildable projection**, not a second source of truth. Per
[ADR-005](../ADRs/005-sqlite-plus-hnsw.md) it holds only `(claim_id, embedding)` pairs and can
be reconstructed in full by scanning the database and re-indexing. So:

- **Back up the database.** That alone preserves everything; nothing is lost if the index is
  gone.
- **The index is optional to back up.** On restore you can either rebuild it from the database
  (a normal offline reindex, per ADR-005) or copy the index file as a speed optimization for
  large stores — and even a slightly stale copy self-heals, since a missing/extra vector only
  means a claim is briefly un-searchable or a dangling hit is filtered.

The one real hazard is a **torn** SQLite file from copying it mid-write, which §3 avoids.

> Adapter note: this is the SQLite story. A Postgres + pgvector adapter (see
> [ADR-020](../ADRs/020-swappable-storage-backends.md)) keeps claims and embeddings in one
> transactional database, so its backup is a single `pg_dump` / WAL PITR with no separate index
> at all. Backup mechanics are adapter-specific.

## 3. Consistency strategy

- **SQLite:** use SQLite's own online snapshot — `VACUUM INTO 'snapshot.db'` or the backup API
  (`.backup`) — which produces a consistent copy even under concurrent writes. Never `cp` a live
  database file. Account for WAL mode (checkpoint as part of the snapshot).
- **Vector index:** because it is a rebuildable projection (§2), it does **not** need a
  point-consistent joint snapshot with the database. Simplest correct approach: back up only the
  database and **rebuild the index on restore**. If a reindex is too slow for a large store, copy
  the index file too as an optimization — no quiescing required, since a stale copy self-heals.
- **Integrity check:** after snapshotting, verify the copy opens and its claim count matches
  before declaring the backup good. A backup that has never been opened is not a backup.

## 4. Two grades of recovery

Backups are one of **two** complementary recovery tools; reach for the lighter one first.

- **The provenance scalpel (surgical).** Because every write is (per the write-path design)
  stamped with author, tier, and timestamp — and dev/low-trust writes carry a `dev_provider`
  marker — most contamination can be removed *targeted*: "delete everything authored by
  `agent:runaway-3` after 14:00," or "drop all `dev_provider` writes." This loses only the bad
  actor's entries, not everyone's day. Prefer it whenever the damage is attributable.
- **Backups (blunt).** When the damage isn't cleanly attributable, or the store is structurally
  corrupt, restore the most recent good snapshot. You lose memory written since that snapshot —
  acceptable for the catastrophic case, and Boswell's confidence/decay model tolerates a gap
  better than a flat datastore would.

## 5. Backup procedure

- **Schedule:** nightly by default (frequency configurable; the durability/lost-work tradeoff is
  the operator's).
- **Retention:** keep N daily snapshots; optionally grandfather-father-son (a few weeklies, a few
  monthlies). Compress snapshots.
- **Location:** at minimum a separate directory or disk from the live store; optionally pushed
  off-box. Local-first does not mean single-disk.
- **Naming:** timestamped, with the schema version recorded, so a restore knows what it is
  loading.

## 6. Restore procedure

Restore is the actual deliverable — design and test it, don't assume it.

1. Stop the instance (or put it in a maintenance mode that rejects writes).
2. Move the current DB + vector index aside (never delete the thing you're replacing).
3. Put the snapshot's database in place, then **rebuild the vector index** from it (or drop in a
   copied index file if you kept one).
4. Restart; verify claim counts and run a sample semantic query to confirm the index and DB
   agree.
5. Keep a **periodic restore-test** in the operator's routine — restore into a scratch instance
   on a schedule and confirm it comes up clean. This is what turns "we have backups" into "we can
   recover."

## 7. Where it lives

A pair of CLI subcommands the operator schedules externally:

- `boswell backup [--out DIR]` — quiesce, snapshot both artifacts consistently, verify, rotate.
- `boswell restore <snapshot>` — the procedure in §6.

Scheduling is left to `cron` or a `systemd` timer (documented examples), **not** a scheduler
built into Boswell. Rationale: least-magic and portable for local deployments; Boswell should not
own a job scheduler it doesn't otherwise need. A built-in scheduled backup worker (alongside the
existing maintenance workers) is a reasonable *later* option, gated behind config.

A `[backup]` config section can hold defaults (output dir, retention, compression); the schedule
itself stays in the operator's cron/timer.

## 8. Future / out of scope for v1

- **Point-in-time recovery.** WAL archiving would allow restoring to an arbitrary moment rather
  than the last nightly snapshot. Deferred; the provenance scalpel covers many of the cases PITR
  otherwise would.
- **Encryption at rest for backups.** Aligns with the aspirational `age`-encrypted config in
  `10-security.md`; snapshots of a memory store are sensitive and should eventually be encrypted.
- **Multi-instance / federated backups.** When federation lands, a consistent cross-instance
  snapshot is its own problem.

## 9. Relationship to existing components

- **Store** (`04-claim-store`) — owns both artifacts; the snapshot/restore primitives live here.
- **Janitor / Synthesizer / Contradiction** (`06`, `07`) — background writers; SQLite's online
  backup captures a consistent database snapshot without stopping them.
- **Storage backends** ([ADR-005](../ADRs/005-sqlite-plus-hnsw.md),
  [ADR-020](../ADRs/020-swappable-storage-backends.md)) — the database is the source of truth and
  the vector index is a rebuildable projection; backup mechanics are adapter-specific.
- **Provenance / write path** (`15-procedural-memory.md`) — enables the surgical recovery grade.
- **Security** (`10-security`) — backup-at-rest encryption belongs with that roadmap.
