//! Paired prediction scoring with whole-group uncertainty and executable adapters.
use crate::{
    code::{self, Check},
    ingest::hash_file,
};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    id: String,
    group: String,
    expected: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prediction {
    id: String,
    prediction: Value,
}
fn records<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Vec<T>> {
    if fs::metadata(p)?.len() > 32 * 1024 * 1024 {
        bail!("lab file limited to 32MiB")
    };
    let text = fs::read_to_string(p)?;
    let mut rows = vec![];
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if rows.len() >= 100000 {
            bail!("100000 record limit")
        };
        rows.push(serde_json::from_str(line)?)
    }
    if rows.is_empty() {
        bail!("empty lab input")
    };
    Ok(rows)
}
fn predictions(p: &Path) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for row in records::<Prediction>(p)? {
        if row.id.is_empty() || out.insert(row.id, row.prediction).is_some() {
            bail!("duplicate/empty prediction ID")
        }
    }
    Ok(out)
}
pub fn compare(data: &Path, baseline: &Path, candidate: &Path, mode: &str) -> Result<Value> {
    if !["exact", "regression"].contains(&mode) {
        bail!("mode must be exact or regression")
    };
    let rows = records::<Example>(data)?;
    let a = predictions(baseline)?;
    let b = predictions(candidate)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut groups: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    let mut wins = 0;
    let mut losses = 0;
    for row in &rows {
        if row.id.is_empty() || row.group.is_empty() || !seen.insert(row.id.clone()) {
            bail!("duplicate/empty example ID or group")
        };
        let old = a.get(&row.id).context("baseline missing example")?;
        let new = b.get(&row.id).context("candidate missing example")?;
        let score = |v: &Value| -> Result<f64> {
            if mode == "exact" {
                Ok(if v == &row.expected { 1. } else { 0. })
            } else {
                let v = v.as_f64().context("numeric prediction required")?;
                let expected = row
                    .expected
                    .as_f64()
                    .context("numeric expectation required")?;
                let e = (v - expected).powi(2);
                if !e.is_finite() {
                    bail!("nonfinite regression error")
                };
                Ok(e)
            }
        };
        let x = score(old)?;
        let y = score(new)?;
        if if mode == "exact" { y > x } else { y < x } {
            wins += 1
        } else if x != y {
            losses += 1
        };
        groups.entry(row.group.clone()).or_default().push((x, y));
    }
    if seen != a.keys().cloned().collect() || seen != b.keys().cloned().collect() {
        bail!("prediction coverage differs from dataset")
    }
    let total = rows.len() as f64;
    let old = if mode == "exact" {
        groups.values().flatten().map(|p| p.0).sum::<f64>() / total
    } else {
        groups.values().flatten().map(|p| p.0 / total).sum::<f64>()
    };
    let new = if mode == "exact" {
        groups.values().flatten().map(|p| p.1).sum::<f64>() / total
    } else {
        groups.values().flatten().map(|p| p.1 / total).sum::<f64>()
    };
    if !old.is_finite() || !new.is_finite() {
        bail!("nonfinite aggregate score")
    }
    let interval = if groups.len() < 2 {
        None
    } else {
        let g: Vec<_> = groups.values().collect();
        let mut state = 7u64;
        let mut deltas = vec![];
        for _ in 0..500 {
            let mut indices = Vec::with_capacity(g.len());
            let mut n = 0usize;
            for _ in 0..g.len() {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let index = (state as usize) % g.len();
                n += g[index].len();
                indices.push(index);
            }
            let mut delta = 0.;
            for index in indices {
                for (a, b) in g[index] {
                    delta += b / n as f64 - a / n as f64;
                }
            }
            if !delta.is_finite() {
                bail!("nonfinite bootstrap delta")
            }
            deltas.push(delta);
        }
        deltas.sort_by(f64::total_cmp);
        Some([deltas[12], deltas[487]])
    };
    Ok(
        json!({"schema_version":1,"metric":if mode=="exact"{"exact_match_accuracy"}else{"mean_squared_error"},"direction":if mode=="exact"{"max"}else{"min"},"n":rows.len(),"groups":groups.len(),"baseline":old,"candidate":new,"candidate_minus_baseline":new-old,"paired_group_bootstrap_ci95":interval,"wins":wins,"losses":losses,"ties":rows.len()-wins-losses,"dataset_sha256":hash_file(data)?,"baseline_predictions_sha256":hash_file(baseline)?,"candidate_predictions_sha256":hash_file(candidate)?,"evaluator_sha256":crate::ingest::hash(include_bytes!("lab.rs")),"scope":"Exact JSON equality or numeric MSE only. Group independence is caller-declared. No semantic quality, hidden model identity, provider cost or general capability inferred."}),
    )
}
pub fn run(data: &Path, adapter: &Path, out: &Path, trusted: bool) -> Result<Value> {
    if !trusted {
        bail!("executable prediction adapter requires --trusted-local; use an isolated operator stage for untrusted code")
    };
    let _: Vec<Example> = records(data)?;
    let adapter_sha256 = hash_file(adapter)?;
    let mut check: Check = serde_json::from_slice(&fs::read(adapter)?)?;
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let data = data.canonicalize()?;
    let identity = hash_file(&data)?;
    let predictions = out.join("predictions.jsonl");
    for arg in &mut check.args {
        *arg = arg
            .replace("{dataset}", &data.to_string_lossy())
            .replace("{predictions}", &predictions.to_string_lossy());
    }
    let result = code::run_observed(
        &out,
        &check,
        "prediction-adapter",
        &check.id,
        Duration::from_secs(300),
        Some(&out.join("logs")),
        None,
    )?;
    if result.status != "passed"
        || hash_file(&data)? != identity
        || hash_file(adapter)? != adapter_sha256
    {
        bail!("adapter failed or changed dataset")
    };
    let _ = compare(&data, &predictions, &predictions, "exact")?;
    let report = json!({"adapter_sha256":adapter_sha256,"dataset_sha256":identity,"predictions_sha256":hash_file(&predictions)?,"execution":result,"cost":"not_measured","scope":"Actual executable adapter; provider/model identity requires additional evidence."});
    fs::write(
        out.join("receipt.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perfect_accuracy_remains_exact() {
        let d = tempfile::tempdir().unwrap();
        let data = d.path().join("data.jsonl");
        let predictions = d.path().join("predictions.jsonl");
        let mut examples = String::new();
        let mut outputs = String::new();
        for i in 0..6 {
            examples += &format!(
                "{}\n",
                json!({"id":i.to_string(),"group":(i/2).to_string(),"expected":true})
            );
            outputs += &format!("{}\n", json!({"id":i.to_string(),"prediction":true}));
        }
        fs::write(&data, examples).unwrap();
        fs::write(&predictions, outputs).unwrap();
        let r = compare(&data, &predictions, &predictions, "exact").unwrap();
        assert_eq!(r["candidate"].as_f64(), Some(1.));
    }
    #[test]
    fn large_finite_regression_values_do_not_become_null() {
        let d = tempfile::tempdir().unwrap();
        let data = d.path().join("data.jsonl");
        let a = d.path().join("a.jsonl");
        let b = d.path().join("b.jsonl");
        fs::write(
            &data,
            concat!(
                r#"{"id":"a","group":"g1","expected":1e154}"#,
                "\n",
                r#"{"id":"b","group":"g2","expected":1e154}"#,
                "\n"
            ),
        )
        .unwrap();
        fs::write(
            &a,
            concat!(
                r#"{"id":"a","prediction":0}"#,
                "\n",
                r#"{"id":"b","prediction":0}"#,
                "\n"
            ),
        )
        .unwrap();
        fs::write(
            &b,
            concat!(
                r#"{"id":"a","prediction":1e154}"#,
                "\n",
                r#"{"id":"b","prediction":1e154}"#,
                "\n"
            ),
        )
        .unwrap();
        let report = compare(&data, &a, &b, "regression").unwrap();
        assert!(report["baseline"].as_f64().unwrap() > 1e307);
        assert!(report["paired_group_bootstrap_ci95"][0]
            .as_f64()
            .unwrap()
            .is_finite());
    }
}
