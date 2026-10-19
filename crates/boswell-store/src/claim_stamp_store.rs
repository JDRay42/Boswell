//! Who asserted a claim (research design §5.2, ADR-023).
//!
//! Procedures and goal edges carry full provenance stamps in
//! `provenance_stamps`. Claims carry only their author: the dossier needs to say
//! which agents backed a group of claims and how many independence roots that
//! comes to, and nothing more of the stamp is known on the claim write path.
//!
//! A claim is stamped once per distinct author. A second author asserting the
//! same claim id is corroboration and adds a row; the same author asserting it
//! again adds nothing.

use crate::{SqliteStore, StoreError};
use boswell_domain::ClaimId;
use rusqlite::params;
use std::collections::HashMap;

/// One author's stamp on a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimStamp {
    /// The author as the transport recorded it, e.g. `writer/sub:explore-3`.
    pub author: String,
    /// When this author first asserted the claim (Unix seconds).
    pub timestamp: u64,
}

impl SqliteStore {
    /// Record that `author` asserted `claim_id`; the body of
    /// [`ClaimStore::stamp_claim`](boswell_domain::traits::ClaimStore::stamp_claim).
    ///
    /// Returns `true` if this author was new to the claim, `false` if it had
    /// already been stamped by them. An empty or whitespace author records
    /// nothing and returns `false`: an unattributed write stays unattributed
    /// rather than gaining an author named "".
    pub(crate) fn insert_claim_stamp(
        &mut self,
        claim_id: ClaimId,
        author: &str,
        timestamp: u64,
    ) -> Result<bool, StoreError> {
        let author = author.trim();
        if author.is_empty() {
            return Ok(false);
        }
        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO claim_stamps (claim_id, author, timestamp) VALUES (?1, ?2, ?3)",
            params![Self::claim_id_to_bytes(claim_id), author, timestamp as i64],
        )?;
        Ok(affected > 0)
    }

    /// Every author that has stamped `claim_id`, earliest first.
    pub fn claim_stamps(&self, claim_id: ClaimId) -> Result<Vec<ClaimStamp>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT author, timestamp FROM claim_stamps WHERE claim_id = ?1 \
             ORDER BY timestamp, author",
        )?;
        let rows = stmt.query_map(params![Self::claim_id_to_bytes(claim_id)], |row| {
            Ok(ClaimStamp {
                author: row.get(0)?,
                timestamp: row.get::<_, i64>(1)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Stamps for many claims at once, keyed by claim id. Claims with no stamps
    /// are absent from the map.
    pub fn claim_stamps_for(
        &self,
        claim_ids: &[ClaimId],
    ) -> Result<HashMap<ClaimId, Vec<ClaimStamp>>, StoreError> {
        let mut out: HashMap<ClaimId, Vec<ClaimStamp>> = HashMap::new();
        for &id in claim_ids {
            let stamps = self.claim_stamps(id)?;
            if !stamps.is_empty() {
                out.insert(id, stamps);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boswell_domain::traits::ClaimStore;
    use boswell_domain::Claim;

    fn store_with_claim() -> (SqliteStore, ClaimId) {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        let id = store
            .assert_claim(Claim::new(
                ClaimId::new(),
                "team".into(),
                "DateParser".into(),
                "is located at".into(),
                "src/util/date.rs".into(),
                (0.8, 0.9),
                "task".into(),
                100,
            ))
            .unwrap();
        (store, id)
    }

    #[test]
    fn a_second_author_adds_a_stamp_and_a_repeat_does_not() {
        let (mut store, id) = store_with_claim();
        assert!(store.stamp_claim(id, "writer/sub:a", 100).unwrap());
        assert!(store.stamp_claim(id, "auditor", 101).unwrap());
        assert!(!store.stamp_claim(id, "writer/sub:a", 102).unwrap());

        let authors: Vec<_> = store
            .claim_stamps(id)
            .unwrap()
            .into_iter()
            .map(|s| s.author)
            .collect();
        assert_eq!(authors, vec!["writer/sub:a", "auditor"]);
    }

    #[test]
    fn an_empty_author_records_nothing() {
        let (mut store, id) = store_with_claim();
        assert!(!store.stamp_claim(id, "  ", 100).unwrap());
        assert!(store.claim_stamps(id).unwrap().is_empty());
    }

    #[test]
    fn forgetting_a_claim_removes_its_stamps() {
        let (mut store, id) = store_with_claim();
        store.stamp_claim(id, "writer", 100).unwrap();
        assert!(store.delete_claim(id).unwrap());
        assert!(store.claim_stamps(id).unwrap().is_empty());
    }

    #[test]
    fn stamping_a_missing_claim_fails() {
        let mut store = SqliteStore::new(":memory:", false, 0).unwrap();
        assert!(store.stamp_claim(ClaimId::new(), "writer", 100).is_err());
    }

    #[test]
    fn stamps_for_many_claims_omit_the_unstamped() {
        let (mut store, stamped) = store_with_claim();
        let unstamped = store
            .assert_claim(Claim::new(
                ClaimId::new(),
                "team".into(),
                "DateParser".into(),
                "provides".into(),
                "ISO 8601 parsing".into(),
                (0.8, 0.9),
                "task".into(),
                100,
            ))
            .unwrap();
        store.stamp_claim(stamped, "writer", 100).unwrap();

        let map = store.claim_stamps_for(&[stamped, unstamped]).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map[&stamped][0].author, "writer");
    }
}
