//! Practical outcome measurements; independent from acceptance readiness.
use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    run_id: String,
    time_to_first_success_ms: Option<u64>,
    recovery_ms: Option<u64>,
    unnecessary_reruns: Option<u64>,
    patch_attempts: Option<u64>,
    patch_regressions: Option<u64>,
    operator_interventions: Option<u64>,
}
fn distribution(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    let n = values.len();
    if n == 0 {
        return json!({"samples":0,"p50":null,"p95":null});
    }
    json!({"samples":n,"p50":values[(n-1)/2],"p95":values[(95*n).div_ceil(100)-1]})
}
pub fn report(path: &Path) -> Result<Value> {
    if fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        bail!("outcome file exceeds 16 MiB")
    }
    let raw = fs::read_to_string(path)?;
    let mut rows = Vec::new();
    let mut ids = BTreeSet::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let row: Observation = serde_json::from_str(line)?;
        if row.run_id.is_empty()
            || !ids.insert(row.run_id.clone())
            || rows.len() >= 100000
            || row.patch_attempts.is_some() != row.patch_regressions.is_some()
            || row
                .patch_regressions
                .zip(row.patch_attempts)
                .is_some_and(|(r, a)| r > a)
        {
            bail!("invalid or duplicate outcome observation")
        }
        rows.push(row);
    }
    if rows.is_empty() {
        bail!("no outcome observations")
    }
    let attempts: u128 = rows
        .iter()
        .filter_map(|r| r.patch_attempts)
        .map(u128::from)
        .sum();
    let regressions: u128 = rows
        .iter()
        .filter_map(|r| r.patch_regressions)
        .map(u128::from)
        .sum();
    Ok(
        json!({"schema_version":1,"input_sha256":crate::ingest::hash_file(path)?,"runs":rows.len(),
        "time_to_first_success_ms":distribution(rows.iter().filter_map(|r|r.time_to_first_success_ms).collect()),
        "recovery_ms":distribution(rows.iter().filter_map(|r|r.recovery_ms).collect()),
        "unnecessary_reruns":distribution(rows.iter().filter_map(|r|r.unnecessary_reruns).collect()),
        "operator_interventions":distribution(rows.iter().filter_map(|r|r.operator_interventions).collect()),
        "patch_regression_rate":if attempts==0 {None} else {Some(regressions as f64/attempts as f64)},
        "patch_observed_runs":rows.iter().filter(|r|r.patch_attempts.is_some()).count(),
        "scope":"caller-recorded operational outcomes; missing values stay unmeasured; independent from readiness or model quality"}),
    )
}
