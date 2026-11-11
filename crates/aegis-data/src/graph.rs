//! Append-only events and versioned graph projections. Correlation is not causality.
use anyhow::{bail, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};
pub struct GraphStore {
    pub db: Connection,
}
impl GraphStore {
    pub fn open(path: &str) -> Result<Self> {
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS fabric_events(seq INTEGER PRIMARY KEY,run_id TEXT NOT NULL,kind TEXT NOT NULL,payload TEXT NOT NULL,at_ms INTEGER NOT NULL,prev_hash TEXT NOT NULL,hash TEXT NOT NULL); CREATE TABLE IF NOT EXISTS fabric_nodes(id TEXT NOT NULL,seq INTEGER NOT NULL,kind TEXT NOT NULL,attrs TEXT NOT NULL,PRIMARY KEY(id,seq)); CREATE TABLE IF NOT EXISTS fabric_edges(id TEXT NOT NULL,seq INTEGER NOT NULL,source TEXT NOT NULL,target TEXT NOT NULL,kind TEXT NOT NULL,attrs TEXT NOT NULL,PRIMARY KEY(id,seq));")?;
        Ok(Self { db })
    }
    pub fn record(&mut self, run_id: &str, kind: &str, payload: Value) -> Result<i64> {
        let at = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prev: String = tx.query_row(
            "SELECT COALESCE((SELECT hash FROM fabric_events ORDER BY seq DESC LIMIT 1),'')",
            [],
            |r| r.get(0),
        )?;
        let digest = crate::ingest::hash(
            serde_json::to_string(&json!([run_id, kind, payload, at, prev]))?.as_bytes(),
        );
        tx.execute("INSERT INTO fabric_events(run_id,kind,payload,at_ms,prev_hash,hash) VALUES (?1,?2,?3,?4,?5,?6)",params![run_id,kind,payload.to_string(),at,prev,digest])?;
        let seq = tx.last_insert_rowid();
        if kind == "node" {
            let id = payload["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("node id required"))?;
            let k = payload["kind"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("node kind required"))?;
            tx.execute(
                "INSERT INTO fabric_nodes VALUES (?1,?2,?3,?4)",
                params![id, seq, k, payload["attributes"].to_string()],
            )?;
        }
        if kind == "edge" {
            let source = payload["source"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("source required"))?;
            let target = payload["target"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("target required"))?;
            let k = payload["kind"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("edge kind required"))?;
            let id = crate::ingest::hash(format!("{source}\n{k}\n{target}").as_bytes());
            tx.execute(
                "INSERT INTO fabric_edges VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    seq,
                    source,
                    target,
                    k,
                    payload["attributes"].to_string()
                ],
            )?;
        }
        tx.commit()?;
        Ok(seq)
    }
    pub fn node(&mut self, run: &str, id: &str, kind: &str, attrs: Value) -> Result<i64> {
        self.record(run, "node", json!({"id":id,"kind":kind,"attributes":attrs}))
    }
    pub fn edge(
        &mut self,
        run: &str,
        source: &str,
        target: &str,
        kind: &str,
        attrs: Value,
    ) -> Result<i64> {
        self.record(
            run,
            "edge",
            json!({"source":source,"target":target,"kind":kind,"attributes":attrs}),
        )
    }
    pub fn events(&self, run: &str, after: i64) -> Result<Vec<Value>> {
        let mut q=self.db.prepare("SELECT seq,kind,payload,at_ms,hash FROM fabric_events WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT 1000")?;
        let rows = q.query_map(params![run, after], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (s, k, p, t, h) = row?;
            out.push(json!({"seq":s,"kind":k,"payload":serde_json::from_str::<Value>(&p)?,"at_ms":t,"hash":h}));
        }
        Ok(out)
    }
    pub fn snapshot(&self, at: Option<i64>) -> Result<Value> {
        let at = at.unwrap_or(self.db.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM fabric_events",
            [],
            |r| r.get(0),
        )?);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut q=self.db.prepare("SELECT id,seq,kind,attrs FROM fabric_nodes n WHERE seq=(SELECT MAX(seq) FROM fabric_nodes WHERE id=n.id AND seq<=?1) ORDER BY id")?;
        for row in q.query_map([at], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (id, seq, k, a) = row?;
            nodes.push(
                json!({"id":id,"seq":seq,"kind":k,"attributes":serde_json::from_str::<Value>(&a)?}),
            );
        }
        let mut q=self.db.prepare("SELECT id,seq,source,target,kind,attrs FROM fabric_edges n WHERE seq=(SELECT MAX(seq) FROM fabric_edges WHERE id=n.id AND seq<=?1) ORDER BY id")?;
        for row in q.query_map([at], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })? {
            let (id, seq, s, t, k, a) = row?;
            edges.push(json!({"id":id,"seq":seq,"source":s,"target":t,"kind":k,"attributes":serde_json::from_str::<Value>(&a)?}));
        }
        Ok(json!({"schema_version":1,"at_sequence":at,"nodes":nodes,"edges":edges}))
    }
    pub fn impact(&self, id: &str) -> Result<Value> {
        let snapshot = self.snapshot(None)?;
        let mut seen = BTreeSet::from([id.to_owned()]);
        let mut queue = VecDeque::from([id.to_owned()]);
        while let Some(target) = queue.pop_front() {
            for e in snapshot["edges"].as_array().unwrap() {
                if e["target"] == target
                    && ["depends_on", "derived_from", "uses"]
                        .contains(&e["kind"].as_str().unwrap_or(""))
                {
                    if let Some(source) = e["source"].as_str() {
                        if seen.insert(source.into()) {
                            if seen.len() > 10000 {
                                bail!("impact traversal budget exceeded")
                            }
                            queue.push_back(source.into());
                        }
                    }
                }
            }
        }
        Ok(
            json!({"changed":id,"potentially_affected":seen,"interpretation":"Recorded dependencies only; potential impact, not proven causality."}),
        )
    }
    pub fn verify(&self) -> Result<u64> {
        let tx = self.db.unchecked_transaction()?;
        let mut q = tx.prepare(
            "SELECT run_id,kind,payload,at_ms,prev_hash,hash,seq FROM fabric_events ORDER BY seq",
        )?;
        let mut previous = String::new();
        let mut count = 0;
        let (mut nodes, mut edges) = (0i64, 0i64);
        for row in q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })? {
            let (run, k, p, t, prev, hash, seq) = row?;
            let payload: Value = serde_json::from_str(&p)?;
            let expected = crate::ingest::hash(
                serde_json::to_string(&json!([run, k, payload, t, prev]))?.as_bytes(),
            );
            if prev != previous || hash != expected {
                bail!("event hash chain invalid")
            };
            if k == "node" {
                nodes += 1;
                let (kind, attrs): (String, String) = tx.query_row(
                    "SELECT kind,attrs FROM fabric_nodes WHERE id=?1 AND seq=?2",
                    params![payload["id"].as_str(), seq],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                if payload["kind"] != kind
                    || payload["attributes"] != serde_json::from_str::<Value>(&attrs)?
                {
                    bail!("node projection integrity failed")
                }
            }
            if k == "edge" {
                edges += 1;
                let source = payload["source"].as_str().unwrap_or("");
                let target = payload["target"].as_str().unwrap_or("");
                let kind = payload["kind"].as_str().unwrap_or("");
                let id = crate::ingest::hash(format!("{source}\n{kind}\n{target}").as_bytes());
                let (s, t, k, a): (String, String, String, String) = tx.query_row(
                    "SELECT source,target,kind,attrs FROM fabric_edges WHERE id=?1 AND seq=?2",
                    params![id, seq],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                if s != source
                    || t != target
                    || k != kind
                    || serde_json::from_str::<Value>(&a)? != payload["attributes"]
                {
                    bail!("edge projection integrity failed")
                }
            }
            previous = hash;
            count += 1;
        }
        let actual_nodes: i64 =
            tx.query_row("SELECT COUNT(*) FROM fabric_nodes", [], |r| r.get(0))?;
        let actual_edges: i64 =
            tx.query_row("SELECT COUNT(*) FROM fabric_edges", [], |r| r.get(0))?;
        if (actual_nodes, actual_edges) != (nodes, edges) {
            bail!("unrecorded graph projection rows")
        }
        drop(q);
        tx.commit()?;
        Ok(count)
    }
}
pub fn extract(source: &std::path::Path, url: &str) -> Result<Value> {
    let raw = std::fs::read(source)?;
    let text = std::str::from_utf8(&raw)?;
    let mut definitions: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        for token in
            line.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.')
        {
            let token = token.trim_matches('.');
            if token.ends_with(".graph") && token.len() > 6 {
                definitions.entry(token.into()).or_default().push(i + 1);
            }
        }
    }
    let defs:Vec<_>=definitions.into_iter().map(|(name,lines)|json!({"id":format!("graph-definition:{name}"),"name":name,"status":"declared","source_lines":lines})).collect();
    Ok(
        json!({"schema_version":1,"source":{"url":url,"sha256":crate::ingest::hash(&raw)},"unique_graph_basenames":defs.len(),"definitions":defs,"scope":"Declaration catalog, not evidence of implemented models or populated graphs."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_event_roundtrip() {
        let mut g = GraphStore::open(":memory:").unwrap();
        g.record(
            "r",
            "metric",
            json!({"seconds":0.003218017422318718,"value":6.806432018056512}),
        )
        .unwrap();
        assert_eq!(g.verify().unwrap(), 1);
    }
    #[test]
    fn versions_impact_and_tamper() {
        let mut g = GraphStore::open(":memory:").unwrap();
        let first = g.node("r", "data", "dataset", json!({"v":1})).unwrap();
        g.node("r", "data", "dataset", json!({"v":2})).unwrap();
        g.node("r", "model", "model", json!({})).unwrap();
        g.edge("r", "model", "data", "depends_on", json!({}))
            .unwrap();
        assert_eq!(
            g.snapshot(Some(first)).unwrap()["nodes"][0]["attributes"]["v"],
            1
        );
        assert!(g.impact("data").unwrap()["potentially_affected"]
            .as_array()
            .unwrap()
            .contains(&json!("model")));
        assert_eq!(g.verify().unwrap(), 4);
        g.db.execute("UPDATE fabric_events SET payload='{}' WHERE seq=1", [])
            .unwrap();
        assert!(g.verify().is_err());
    }
}
