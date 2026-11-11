//! Acceptance coverage from hash-pinned evidence. No subjective product rating.
use crate::ingest::hash_file;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    areas: Vec<Area>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Area {
    id: String,
    title: String,
    checks: Vec<Check>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    id: String,
    description: String,
    evidence: Option<Evidence>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    path: String,
    sha256: String,
    pointer: String,
    equals: Value,
}
pub fn report(root: &Path, registry: &Path) -> Result<Value> {
    let root = root.canonicalize()?;
    let source = fs::read(registry)?;
    if source.len() > 1024 * 1024 {
        bail!("readiness registry too large")
    }
    let registry: Registry = serde_json::from_slice(&source)?;
    if registry.schema_version != 1 || registry.areas.is_empty() || registry.areas.len() > 100 {
        bail!("invalid readiness registry")
    }
    let mut ids = BTreeSet::new();
    let mut areas = vec![];
    let mut passed = 0;
    let mut total = 0;
    let mut complete = 0;
    let mut invalid = 0;
    for area in registry.areas {
        if !ids.insert(area.id.clone()) || area.checks.is_empty() || area.checks.len() > 100 {
            bail!("duplicate area or invalid check count")
        }
        let mut checks = vec![];
        let mut local = 0;
        let mut check_ids = BTreeSet::new();
        for check in area.checks {
            if !check_ids.insert(check.id.clone()) {
                bail!("duplicate readiness check")
            };
            total += 1;
            let (state, detail) = match check.evidence {
                None => ("unverified", "No evidence attached".to_string()),
                Some(e) => {
                    if e.path.is_empty()
                        || !Path::new(&e.path)
                            .components()
                            .all(|c| matches!(c, Component::Normal(_)))
                        || e.sha256.len() != 64
                        || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                        || !e.pointer.starts_with('/')
                    {
                        bail!("invalid evidence reference")
                    }
                    let path = root.join(&e.path);
                    match path.canonicalize() {
                        Err(_) => ("missing", e.path),
                        Ok(path) => {
                            if !path.starts_with(&root)
                                || !path.is_file()
                                || fs::metadata(&path)?.len() > 16 * 1024 * 1024
                            {
                                bail!("evidence escaped root or exceeds limit")
                            }
                            if hash_file(&path)? != e.sha256 {
                                ("invalid_hash", e.path)
                            } else {
                                let value: Value = serde_json::from_slice(&fs::read(path)?)
                                    .context("evidence must be JSON")?;
                                if value.pointer(&e.pointer) == Some(&e.equals) {
                                    ("verified", e.path)
                                } else {
                                    ("measurement_mismatch", e.path)
                                }
                            }
                        }
                    }
                }
            };
            if state != "verified" && state != "unverified" {
                invalid += 1;
            }
            if state == "verified" {
                passed += 1;
                local += 1
            };
            checks.push(json!({"id":check.id,"description":check.description,"status":state,"evidence":detail}));
        }
        let all = local == checks.len();
        if all {
            complete += 1
        };
        areas.push(json!({"id":area.id,"title":area.title,"status":if all{"verified"}else if local>0{"partial"}else{"unverified"},"verified_checks":local,"checks":checks}));
    }
    Ok(
        json!({"schema_version":1,"registry_sha256":format!("{:x}",sha2::Sha256::digest(&source)),"verified_checks":passed,"invalid_evidence_checks":invalid,"total_checks":total,"fully_verified_areas":complete,"total_areas":areas.len(),"areas":areas,"scope":"Coverage of this declared acceptance registry only; not a benchmark, security certification or 10/10 product score."}),
    )
}
use sha2::Digest;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cannot_promote_tampered_or_missing_evidence() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let e = root.join("e.json");
        fs::write(&e, "{\"passed\":true}").unwrap();
        let r = root.join("registry.json");
        fs::write(&r,serde_json::to_vec(&json!({"schema_version":1,"areas":[{"id":"a","title":"A","checks":[{"id":"proof","description":"proof","evidence":{"path":"e.json","sha256":hash_file(&e).unwrap(),"pointer":"/passed","equals":true}},{"id":"future","description":"not built","evidence":null}]}]})).unwrap()).unwrap();
        assert_eq!(report(root, &r).unwrap()["verified_checks"], 1);
        fs::write(e, "{\"passed\":false}").unwrap();
        assert_eq!(report(root, &r).unwrap()["verified_checks"], 0);
    }
}
