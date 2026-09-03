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

## 2. What must be backed up — two coupled artifacts

A Boswell instance's durable state is **not a single file.** A correct backup must capture
both, at a **consistent logical point**, or a restore will be subtly broken:

1. **The SQLite database** — claims, relationships, provenance, and the confidence cache
   (`boswell-store`, `schema.sql`).
2. **The HNSW vector index** — maintained in a **separate, memory-mapped file** alongside the
   database (`boswell-store/src/vector_index.rs`; see the note in `schema.sql`). It holds the
   embeddings that power semantic search.

The failure mode of getting this wrong is quiet: if the two are snapshotted at different
points, a restore yields a vector index that points at claims the database no longer contains
(or misses ones it does), so semantic search returns ghosts or gaps while structured queries
look fine. A naive `cp` of a live database also risks a **torn** file. So the backup path must
be deliberate, not a filesystem copy.

## 3. Consistency strategy

- **SQLite:** use SQLite's own online snapshot — `VACUUM INTO 'snapshot.db'` or the backup API
  (`.backup`) — which produces a consistent copy even under concurrent writes. Never `cp` a live
  database file. Account for WAL mode (checkpoint as part of the snapshot).
- **Vector index:** snapshot it at the **same** logical point as the database. The instance runs
  opt-in maintenance workers (Janitor, Synthesizer, Contradiction detector) that mutate the store
  in the background, so the backup must briefly **quiesce writers** — pause those workers and let
  in-flight writes drain — take both snapshots, then resume. A short, well-defined pause is
  acceptable for a nightly job; the alternative (a checkpoint/generation marker the index and DB
  share) is a larger change and can come later.
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
3. Put the snapshot's DB and vector index in place **as a pair**.
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
- **Janitor / Synthesizer / Contradiction** (`06`, `07`) — the background writers that must be
  quiesced for a consistent snapshot.
- **Provenance / write path** (`15-procedural-memory.md`) — enables the surgical recovery grade.
- **Security** (`10-security`) — backup-at-rest encryption belongs with that roadmap.
