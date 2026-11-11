//! SQLite task leasing: idempotent enqueue, expiring leases, fencing tokens.
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Lease {
    pub id: String,
    pub worker: String,
    pub epoch: i64,
    pub expires: i64,
}
pub struct Ledger {
    db: Connection,
}
impl Ledger {
    pub fn open(path: &str) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,payload TEXT NOT NULL,state TEXT NOT NULL,worker TEXT,epoch INTEGER NOT NULL DEFAULT 0,expires INTEGER NOT NULL DEFAULT 0,result TEXT);")?;
        let columns: Vec<String> = db
            .prepare("PRAGMA table_info(jobs)")?
            .query_map([], |r| r.get(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !columns.iter().any(|c| c == "max_attempts") {
            db.execute(
                "ALTER TABLE jobs ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 3",
                [],
            )?;
        }
        Ok(Self { db })
    }
    pub fn enqueue(&self, id: &str, payload: &str) -> Result<bool> {
        if id.is_empty() {
            bail!("empty task ID")
        }
        self.db.execute(
            "INSERT OR IGNORE INTO jobs(id,payload,state) VALUES (?1,?2,'queued')",
            params![id, payload],
        )?;
        let old: String = self
            .db
            .query_row("SELECT payload FROM jobs WHERE id=?1", [id], |r| r.get(0))?;
        if old != payload {
            bail!("idempotency key reused with different payload")
        }
        Ok(self.db.changes() > 0)
    }
    pub fn claim(&mut self, worker: &str, now: i64, ttl: i64) -> Result<Option<Lease>> {
        if worker.is_empty() || ttl < 1 || now < 0 {
            bail!("invalid lease")
        }
        let expires = now
            .checked_add(ttl)
            .ok_or_else(|| anyhow::anyhow!("lease overflow"))?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE jobs SET state='failed' WHERE state='leased' AND expires<=?1 AND epoch>=max_attempts", [now])?;
        let row:Option<(String,i64)>=tx.query_row("SELECT id,epoch FROM jobs WHERE epoch<max_attempts AND (state='queued' OR (state='leased' AND expires<=?1)) AND (CASE WHEN json_valid(payload) THEN json_extract(payload,'$.required_worker') ELSE NULL END IS NULL OR CASE WHEN json_valid(payload) THEN json_extract(payload,'$.required_worker') ELSE NULL END=?2) ORDER BY id LIMIT 1",params![now,worker],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let lease = if let Some((id, old)) = row {
            let epoch = old
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("epoch overflow"))?;
            tx.execute(
                "UPDATE jobs SET state='leased',worker=?2,epoch=?3,expires=?4 WHERE id=?1",
                params![id, worker, epoch, expires],
            )?;
            Some(Lease {
                id,
                worker: worker.into(),
                epoch,
                expires,
            })
        } else {
            None
        };
        tx.commit()?;
        Ok(lease)
    }
    pub fn renew(&self, lease: &Lease, now: i64, ttl: i64) -> Result<i64> {
        if now < 0 || ttl < 1 {
            bail!("invalid heartbeat")
        }
        let expires = now
            .checked_add(ttl)
            .ok_or_else(|| anyhow::anyhow!("lease overflow"))?;
        let changed=self.db.execute("UPDATE jobs SET expires=?4 WHERE id=?1 AND worker=?2 AND epoch=?3 AND state='leased' AND expires>?5",params![lease.id,lease.worker,lease.epoch,expires,now])?;
        if changed != 1 {
            bail!("stale/expired heartbeat")
        }
        Ok(expires)
    }
    pub fn cancel(&self, id: &str) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE jobs SET state='cancelled' WHERE id=?1 AND state IN ('queued','leased')",
            [id],
        )? == 1)
    }
    pub fn inspect(&self, id: &str) -> Result<serde_json::Value> {
        let (payload, state, worker, epoch, expires, result): (
            String,
            String,
            Option<String>,
            i64,
            i64,
            Option<String>,
        ) = self.db.query_row(
            "SELECT payload,state,worker,epoch,expires,result FROM jobs WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
        Ok(
            serde_json::json!({"id":id,"payload":payload,"state":state,"worker":worker,"epoch":epoch,"expires":expires,"result":result}),
        )
    }
    pub fn fail(&self, lease: &Lease, now: i64, reason: &str) -> Result<()> {
        let n=self.db.execute("UPDATE jobs SET state='failed',result=?4 WHERE id=?1 AND worker=?2 AND epoch=?3 AND state='leased' AND expires>?5",params![lease.id,lease.worker,lease.epoch,reason,now])?;
        if n != 1 {
            bail!("stale failure")
        };
        Ok(())
    }
    pub fn complete(&self, lease: &Lease, now: i64, result: &str) -> Result<()> {
        let changed=self.db.execute("UPDATE jobs SET state='done',result=?4 WHERE id=?1 AND worker=?2 AND epoch=?3 AND state='leased' AND expires>?5",params![lease.id,lease.worker,lease.epoch,result,now])?;
        if changed != 1 {
            bail!("stale, expired or already completed lease")
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targeted_claims_preserve_worker_admission() {
        let mut l = Ledger::open(":memory:").unwrap();
        l.enqueue("target", r#"{"required_worker":"alpha"}"#)
            .unwrap();
        assert!(l.claim("beta", 100, 10).unwrap().is_none());
        assert_eq!(l.claim("alpha", 100, 10).unwrap().unwrap().id, "target");
    }
    #[test]
    fn idempotency_conflict() {
        let l = Ledger::open(":memory:").unwrap();
        assert!(l.enqueue("a", "x").unwrap());
        assert!(!l.enqueue("a", "x").unwrap());
        assert!(l.enqueue("a", "y").is_err());
    }
    #[test]
    fn heartbeat_and_cancel() {
        let mut l = Ledger::open(":memory:").unwrap();
        l.enqueue("a", "x").unwrap();
        let lease = l.claim("one", 100, 10).unwrap().unwrap();
        assert_eq!(l.renew(&lease, 105, 20).unwrap(), 125);
        assert!(l.claim("two", 111, 10).unwrap().is_none());
        assert!(l.cancel("a").unwrap());
        assert!(l.renew(&lease, 112, 20).is_err());
        assert!(l.complete(&lease, 112, "late").is_err());
        assert!(l.claim("two", 130, 10).unwrap().is_none());
    }
    #[test]
    fn expired_attempt_budget_is_terminal() {
        let mut l = Ledger::open(":memory:").unwrap();
        l.enqueue("budget", "x").unwrap();
        for n in 0..3 {
            assert_eq!(
                l.claim("worker", 100 + n * 10, 10).unwrap().unwrap().epoch,
                n + 1
            );
        }
        assert!(l.claim("worker", 130, 10).unwrap().is_none());
        assert_eq!(l.inspect("budget").unwrap()["state"], "failed");
    }
    #[test]
    fn stale_worker_cannot_commit() {
        let mut l = Ledger::open(":memory:").unwrap();
        l.enqueue("a", "x").unwrap();
        let first = l.claim("one", 100, 10).unwrap().unwrap();
        assert!(l.claim("two", 105, 10).unwrap().is_none());
        let second = l.claim("two", 110, 10).unwrap().unwrap();
        assert!(l.complete(&first, 111, "stale").is_err());
        l.complete(&second, 111, "ok").unwrap();
        assert!(l.claim("three", 112, 10).unwrap().is_none());
    }
}
