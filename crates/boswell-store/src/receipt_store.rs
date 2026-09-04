//! Execution-receipt lifecycle — the effectiveness-capture ledger (procedural
//! memory, Phase 5).
//!
//! Retrieving a procedure for execution carries an obligation to report the
//! outcome (design §3.3). This module persists that contract: [`issue_receipt`]
//! records a pending receipt; [`report_receipt`] applies the executor's outcome
//! (as a gatekept, provenance-stamped write) and closes the receipt; and
//! [`expire_receipts`] enforces **"silence is not success"** — an unreported,
//! expired receipt is marked `expired` and counts as an `unknown` against the
//! procedure's effectiveness.
//!
//! Capture is driven operationally by hooks (`SubagentStop`/`Stop`/`PostToolUse`);
//! see `examples/claude-code-hooks`. The transport that carries a hook's report to
//! [`report_receipt`] (a CLI/gateway endpoint) is deferred with the other
//! procedure endpoints.
//!
//! [`issue_receipt`]: SqliteStore::issue_receipt
//! [`report_receipt`]: SqliteStore::report_receipt
//! [`expire_receipts`]: SqliteStore::expire_receipts

use crate::provenance_store::StampedReportOutcome;
use crate::{SqliteStore, StoreError};
use boswell_domain::{
    ExecutionReceipt, OutcomeReport, ProcedureId, ProvenanceStamp, ReceiptStatus,
};
use rusqlite::{params, OptionalExtension};

/// A receipt as read back from the ledger, with its lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReceipt {
    /// The receipt.
    pub receipt: ExecutionReceipt,
    /// Its current status.
    pub status: ReceiptStatus,
}

/// The result of reporting against a receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptReportOutcome {
    /// The gatekept report application, if it ran (`None` if the procedure no
    /// longer exists).
    pub applied: Option<StampedReportOutcome>,
    /// Whether the receipt was already closed (reported or expired) — in which
    /// case nothing was applied.
    pub already_final: bool,
}

fn id_bytes(id: ProcedureId) -> Vec<u8> {
    id.value().to_be_bytes().to_vec()
}

fn id_from_bytes(bytes: &[u8]) -> Result<ProcedureId, StoreError> {
    if bytes.len() != 16 {
        return Err(StoreError::InvalidData(format!(
            "Expected 16 bytes for a receipt/procedure id, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(bytes);
    Ok(ProcedureId::from_value(u128::from_be_bytes(arr)))
}

impl SqliteStore {
    /// Record a newly issued execution receipt as `pending` (design §3.3).
    ///
    /// Build the receipt with [`ExecutionReceipt::issue`]. Returns the receipt id.
    pub fn issue_receipt(&mut self, receipt: &ExecutionReceipt) -> Result<ProcedureId, StoreError> {
        self.conn.execute(
            "INSERT INTO execution_receipts (
                receipt_id, procedure_id, version, issued_to, task_id, session_id,
                issued_at, expires_at, report_to, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending')",
            params![
                id_bytes(receipt.receipt_id),
                id_bytes(receipt.procedure_id),
                receipt.version,
                &receipt.issued_to,
                &receipt.task_id,
                &receipt.session_id,
                receipt.issued_at as i64,
                receipt.expires_at as i64,
                &receipt.report_to,
            ],
        )?;
        Ok(receipt.receipt_id)
    }

    /// Fetch a receipt by id, or `None` if it does not exist.
    pub fn get_receipt(
        &self,
        receipt_id: ProcedureId,
    ) -> Result<Option<StoredReceipt>, StoreError> {
        self.conn
            .query_row(
                "SELECT receipt_id, procedure_id, version, issued_to, task_id, session_id, \
                 issued_at, expires_at, report_to, status FROM execution_receipts \
                 WHERE receipt_id = ?1",
                params![id_bytes(receipt_id)],
                Self::row_to_receipt,
            )
            .optional()?
            .transpose()
    }

    /// Apply an outcome report against a pending receipt (design §3.3).
    ///
    /// Resolves the receipt to its procedure, applies the report as a gatekept,
    /// provenance-stamped write ([`SqliteStore::apply_procedure_report_stamped`]),
    /// and marks the receipt `reported`. Returns `None` if no such receipt exists;
    /// a receipt already `reported`/`expired` is left untouched
    /// (`already_final = true`). `now` is Unix ms.
    pub fn report_receipt(
        &mut self,
        receipt_id: ProcedureId,
        report: &OutcomeReport,
        stamp: &ProvenanceStamp,
        now: u64,
    ) -> Result<Option<ReceiptReportOutcome>, StoreError> {
        let Some(stored) = self.get_receipt(receipt_id)? else {
            return Ok(None);
        };
        if stored.status != ReceiptStatus::Pending {
            return Ok(Some(ReceiptReportOutcome {
                applied: None,
                already_final: true,
            }));
        }

        let applied =
            self.apply_procedure_report_stamped(stored.receipt.procedure_id, report, stamp, now)?;
        self.set_receipt_status(receipt_id, ReceiptStatus::Reported)?;
        Ok(Some(ReceiptReportOutcome {
            applied,
            already_final: false,
        }))
    }

    /// Expire pending receipts whose deadline has passed, enforcing "silence is
    /// not success" (design §3.3): each is marked `expired` and records an
    /// `unknown` against its procedure. Returns the number expired. `now` is Unix ms.
    pub fn expire_receipts(&mut self, now: u64) -> Result<usize, StoreError> {
        // Collect the (receipt_id, procedure_id) of overdue pending receipts.
        let overdue: Vec<(ProcedureId, ProcedureId)> = {
            let mut stmt = self.conn.prepare(
                "SELECT receipt_id, procedure_id FROM execution_receipts \
                 WHERE status = 'pending' AND expires_at <= ?1",
            )?;
            let rows = stmt
                .query_map(params![now as i64], |row| {
                    let rid: Vec<u8> = row.get(0)?;
                    let pid: Vec<u8> = row.get(1)?;
                    Ok((rid, pid))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            rows.into_iter()
                .map(|(rid, pid)| Ok((id_from_bytes(&rid)?, id_from_bytes(&pid)?)))
                .collect::<Result<Vec<_>, StoreError>>()?
        };

        let mut expired = 0;
        for (receipt_id, procedure_id) in overdue {
            if let Some(mut procedure) = self.get_procedure(procedure_id)? {
                procedure.record_unknown(now);
                self.upsert_procedure(&procedure)?;
            }
            self.set_receipt_status(receipt_id, ReceiptStatus::Expired)?;
            expired += 1;
        }
        Ok(expired)
    }

    /// Count receipts currently in `status`.
    pub fn count_receipts(&self, status: ReceiptStatus) -> Result<usize, StoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM execution_receipts WHERE status = ?1",
            params![status.as_str()],
            |row| row.get(0),
        )?;
        Ok(n as usize)
    }

    fn set_receipt_status(
        &self,
        receipt_id: ProcedureId,
        status: ReceiptStatus,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE execution_receipts SET status = ?1 WHERE receipt_id = ?2",
            params![status.as_str(), id_bytes(receipt_id)],
        )?;
        Ok(())
    }

    fn row_to_receipt(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<Result<StoredReceipt, StoreError>> {
        let decode = |row: &rusqlite::Row<'_>| -> Result<StoredReceipt, StoreError> {
            let receipt_id = id_from_bytes(&row.get::<_, Vec<u8>>(0)?)?;
            let procedure_id = id_from_bytes(&row.get::<_, Vec<u8>>(1)?)?;
            let status_str: String = row.get(9)?;
            let status = ReceiptStatus::parse(&status_str).ok_or_else(|| {
                StoreError::InvalidData(format!("Unknown receipt status: {}", status_str))
            })?;
            Ok(StoredReceipt {
                receipt: ExecutionReceipt {
                    receipt_id,
                    procedure_id,
                    version: row.get(2)?,
                    issued_to: row.get(3)?,
                    task_id: row.get(4)?,
                    session_id: row.get(5)?,
                    issued_at: row.get::<_, i64>(6)? as u64,
                    expires_at: row.get::<_, i64>(7)? as u64,
                    report_to: row.get(8)?,
                },
                status,
            })
        };
        Ok(decode(row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::{
        Assurance, Authority, BodyFormat, DelegationChain, EvidenceType, FailureMode, Op,
        Procedure, ProcedureSource, Tier,
    };

    const NOW: u64 = 1_700_000_000_000;

    fn store() -> SqliteStore {
        SqliteStore::new(":memory:", false, 0).unwrap()
    }

    fn procedure(tier: Tier) -> Procedure {
        Procedure {
            id: ProcedureId::new(),
            namespace: "person:jd".into(),
            name: "p".into(),
            version: 1,
            supersedes: None,
            is_current: true,
            source: ProcedureSource::Authored,
            goal: "g".into(),
            intent: "i".into(),
            tags: vec![],
            parameters: vec![],
            preconditions: vec![],
            required_tools: vec![],
            postconditions: vec![],
            est_duration_sec: None,
            usage_notes: String::new(),
            context_tags: vec![],
            body_format: BodyFormat::Prose,
            content_type: "text/plain".into(),
            body: "b".into(),
            tier,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            unknown_count: 0,
            last_used_at: None,
            created_at: NOW,
            updated_at: NOW,
            stale_at: None,
        }
    }

    fn stamp() -> ProvenanceStamp {
        ProvenanceStamp {
            author: "agent:worker".into(),
            delegation_chain: DelegationChain(vec!["agent:worker".into()]),
            authority: Authority {
                namespaces: vec!["person:jd".into()],
                max_tier: Tier::Task,
                ops: vec![Op::Write],
            },
            evidence: EvidenceType::Observed,
            assurance: Assurance::Verified,
            task_id: None,
            session_id: None,
            timestamp: NOW,
            dev_provider: false,
        }
    }

    #[test]
    fn issue_get_roundtrip() {
        let mut store = store();
        let proc = procedure(Tier::Task);
        store.upsert_procedure(&proc).unwrap();
        let receipt = ExecutionReceipt::issue(&proc, "agent:worker", NOW, 1000);
        store.issue_receipt(&receipt).unwrap();

        let got = store.get_receipt(receipt.receipt_id).unwrap().unwrap();
        assert_eq!(got.receipt, receipt);
        assert_eq!(got.status, ReceiptStatus::Pending);
        assert_eq!(store.count_receipts(ReceiptStatus::Pending).unwrap(), 1);
    }

    #[test]
    fn report_applies_and_closes_receipt() {
        let mut store = store();
        let proc = procedure(Tier::Task);
        store.upsert_procedure(&proc).unwrap();
        let receipt = ExecutionReceipt::issue(&proc, "agent:worker", NOW, 1000);
        store.issue_receipt(&receipt).unwrap();

        let outcome = store
            .report_receipt(
                receipt.receipt_id,
                &OutcomeReport::success(receipt.receipt_id),
                &stamp(),
                NOW + 10,
            )
            .unwrap()
            .unwrap();
        assert!(!outcome.already_final);
        assert!(outcome.applied.unwrap().effect.unwrap().counted_as_success);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().success_count,
            1
        );
        assert_eq!(
            store
                .get_receipt(receipt.receipt_id)
                .unwrap()
                .unwrap()
                .status,
            ReceiptStatus::Reported
        );
    }

    #[test]
    fn double_report_is_noop() {
        let mut store = store();
        let proc = procedure(Tier::Task);
        store.upsert_procedure(&proc).unwrap();
        let receipt = ExecutionReceipt::issue(&proc, "agent:worker", NOW, 1000);
        store.issue_receipt(&receipt).unwrap();
        store
            .report_receipt(
                receipt.receipt_id,
                &OutcomeReport::failure(receipt.receipt_id, FailureMode::BadResult),
                &stamp(),
                NOW + 10,
            )
            .unwrap();
        // A second report does nothing.
        let second = store
            .report_receipt(
                receipt.receipt_id,
                &OutcomeReport::success(receipt.receipt_id),
                &stamp(),
                NOW + 20,
            )
            .unwrap()
            .unwrap();
        assert!(second.already_final);
        let p = store.get_procedure(proc.id).unwrap().unwrap();
        assert_eq!(p.failure_count, 1);
        assert_eq!(p.success_count, 0);
    }

    #[test]
    fn report_unknown_receipt_is_none() {
        let mut store = store();
        assert!(store
            .report_receipt(
                ProcedureId::new(),
                &OutcomeReport::success(ProcedureId::new()),
                &stamp(),
                NOW,
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn expire_records_unknown_and_marks_expired() {
        let mut store = store();
        let proc = procedure(Tier::Task);
        store.upsert_procedure(&proc).unwrap();
        // A receipt that has already expired, and one still valid.
        let overdue = ExecutionReceipt::issue(&proc, "agent:worker", NOW - 2000, 1000);
        let live = ExecutionReceipt::issue(&proc, "agent:worker", NOW, 10_000);
        store.issue_receipt(&overdue).unwrap();
        store.issue_receipt(&live).unwrap();

        let expired = store.expire_receipts(NOW).unwrap();
        assert_eq!(expired, 1);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().unknown_count,
            1
        );
        assert_eq!(
            store
                .get_receipt(overdue.receipt_id)
                .unwrap()
                .unwrap()
                .status,
            ReceiptStatus::Expired
        );
        assert_eq!(
            store.get_receipt(live.receipt_id).unwrap().unwrap().status,
            ReceiptStatus::Pending
        );

        // Expiring again is idempotent (the overdue one is already closed).
        assert_eq!(store.expire_receipts(NOW).unwrap(), 0);
    }

    #[test]
    fn expire_does_not_touch_reported_receipts() {
        let mut store = store();
        let proc = procedure(Tier::Task);
        store.upsert_procedure(&proc).unwrap();
        let receipt = ExecutionReceipt::issue(&proc, "agent:worker", NOW - 5000, 1000);
        store.issue_receipt(&receipt).unwrap();
        store
            .report_receipt(
                receipt.receipt_id,
                &OutcomeReport::success(receipt.receipt_id),
                &stamp(),
                NOW,
            )
            .unwrap();
        // Even though its deadline passed, a reported receipt is not expired.
        assert_eq!(store.expire_receipts(NOW).unwrap(), 0);
        assert_eq!(
            store.get_procedure(proc.id).unwrap().unwrap().unknown_count,
            0
        );
    }
}
