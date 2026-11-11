//! Shared provenance controls, admission quotes and contained incident-drill adapters.
use crate::{graph::GraphStore, ingest::hash_file};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
pub fn link(db: &str, output: &Path, inputs: &[std::path::PathBuf]) -> Result<Value> {
    if inputs.is_empty() || inputs.len() > 10000 {
        bail!("recorded inputs required")
    };
    let mut g = GraphStore::open(db)?;
    let snapshot = g.snapshot(None)?;
    let add = |g: &mut GraphStore, path: &Path| -> Result<String> {
        let sha = hash_file(path)?;
        let id = format!("artifact:{sha}");
        let quarantined = snapshot["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == id && n["attributes"]["status"] == "quarantined");
        g.node("lineage",&id,"artifact",json!({"path":path,"sha256":sha,"status":if quarantined{"quarantined"}else{"observed"}}))?;
        Ok(id)
    };
    let result = add(&mut g, output)?;
    for input in inputs {
        let source = add(&mut g, input)?;
        g.edge(
            "lineage",
            &result,
            &source,
            "derived_from",
            json!({"relationship":"declared_dependency","not_causal_proof":true}),
        )?;
    }
    Ok(json!({"artifact":result,"inputs":inputs.len()}))
}
pub fn quarantine(db: &str, sha: &str, reason: &str) -> Result<Value> {
    if sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || reason.is_empty()
        || reason.len() > 4096
    {
        bail!("valid SHA and bounded reason required")
    };
    let mut g = GraphStore::open(db)?;
    let id = format!("artifact:{sha}");
    let impact = g.impact(&id)?;
    let snapshot = g.snapshot(None)?;
    if !snapshot["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["id"] == id)
    {
        bail!("unknown provenance source")
    }
    for affected in impact["potentially_affected"].as_array().unwrap() {
        let id = affected.as_str().unwrap();
        if id.starts_with("artifact:") {
            let mut attrs = snapshot["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == id)
                .map(|n| n["attributes"].clone())
                .unwrap_or_else(|| json!({}));
            attrs["status"] = json!("quarantined");
            attrs["reason"] = json!(reason);
            g.node("lineage", id, "artifact", attrs)?;
        }
    }
    g.record(
        "lineage",
        "quarantine",
        json!({"source":sha,"reason":reason,"impact":impact}),
    )?;
    Ok(impact)
}
pub fn allow(db: &str, sha: &str) -> Result<Value> {
    let g = GraphStore::open(db)?;
    g.verify()?;
    let snapshot = g.snapshot(None)?;
    let id = format!("artifact:{sha}");
    if !snapshot["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["id"] == id)
    {
        bail!("unknown provenance")
    };
    let mut todo = vec![id];
    let mut seen = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        };
        if seen.len() > 10000 {
            bail!("lineage budget exceeded")
        };
        if snapshot["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == id && n["attributes"]["status"] == "quarantined")
        {
            bail!("artifact or recorded input quarantined")
        };
        for edge in snapshot["edges"].as_array().unwrap() {
            if edge["source"] == id
                && ["derived_from", "uses", "depends_on"]
                    .contains(&edge["kind"].as_str().unwrap_or(""))
            {
                todo.push(edge["target"].as_str().unwrap().into());
            }
        }
    }
    Ok(
        json!({"allowed":true,"artifact":sha,"checked_dependencies":seen.len(),"scope":"Recorded dependencies only; no deletion or model unlearning claim."}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    cpu: u64,
    memory_mib: u64,
    gpu_count: u64,
    gpu_memory_mib: u64,
    duration_seconds: u64,
    max_cost_micros: u64,
    capabilities: Vec<String>,
    image: Option<String>,
    max_age_seconds: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Worker {
    id: String,
    observed_at_unix: u64,
    cpu_available: u64,
    memory_mib_available: u64,
    gpu_count_available: u64,
    gpu_memory_mib_available: u64,
    price_micros_per_second: u64,
    capabilities: Vec<String>,
    images: Vec<String>,
}
pub fn quote(request: &Path, inventory: &Path) -> Result<Value> {
    if fs::metadata(request)?.len() > 1024 * 1024
        || fs::metadata(inventory)?.len() > 16 * 1024 * 1024
    {
        bail!("admission input budget exceeded")
    }

    let r: Request = serde_json::from_slice(&fs::read(request)?)?;
    let workers: Vec<Worker> = serde_json::from_slice(&fs::read(inventory)?)?;
    if r.cpu == 0
        || r.memory_mib == 0
        || r.duration_seconds == 0
        || r.max_age_seconds == 0
        || r.max_age_seconds > 3600
        || workers.is_empty()
        || workers.len() > 10000
    {
        bail!("invalid admission request")
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut ids = BTreeSet::new();
    let mut eligible = vec![];
    let mut rejected = vec![];
    for w in workers {
        if w.id.is_empty()
            || !ids.insert(w.id.clone())
            || !w
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            bail!("duplicate/invalid worker identity")
        };
        let mut reasons = vec![];
        if w.observed_at_unix > now + 5
            || now.saturating_sub(w.observed_at_unix) > r.max_age_seconds
        {
            reasons.push("stale_inventory")
        };
        if w.cpu_available < r.cpu
            || w.memory_mib_available < r.memory_mib
            || w.gpu_count_available < r.gpu_count
            || w.gpu_memory_mib_available < r.gpu_memory_mib
        {
            reasons.push("insufficient_resources")
        };
        if !r.capabilities.iter().all(|c| w.capabilities.contains(c)) {
            reasons.push("missing_capability")
        };
        if r.image
            .as_ref()
            .is_some_and(|image| !w.images.contains(image))
        {
            reasons.push("image_unavailable")
        };
        let cost = w
            .price_micros_per_second
            .checked_mul(r.duration_seconds)
            .context("cost overflow")?;
        if cost > r.max_cost_micros {
            reasons.push("cost_limit")
        };
        if reasons.is_empty() {
            eligible.push(json!({"worker":w.id,"estimated_cost_micros":cost}))
        } else {
            rejected.push(json!({"worker":w.id,"reasons":reasons}))
        }
    }
    eligible.sort_by(|a, b| {
        a["estimated_cost_micros"]
            .as_u64()
            .cmp(&b["estimated_cost_micros"].as_u64())
            .then_with(|| a["worker"].as_str().cmp(&b["worker"].as_str()))
    });
    Ok(
        json!({"chosen":eligible.first(),"eligible":eligible,"rejected":rejected,"request_sha256":hash_file(request)?,"inventory_sha256":hash_file(inventory)?,"scope":"Admission quote from supplied inventory, not live reservation, billing or provider provisioning. Use worker-enqueue --worker to enforce chosen identity."}),
    )
}
pub fn drill(
    workspace: &Path,
    binary: &Path,
    out: &Path,
    scenario: &str,
    python: &str,
    trusted: bool,
) -> Result<Value> {
    if !trusted {
        bail!("contained verification drills require --trusted-local")
    };
    let script = match scenario {
        "worker-loss" => "scripts/verify_workers.py",
        "remote-disconnect" => "scripts/verify_remote.py",
        _ => bail!("unknown contained drill"),
    };
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let script = workspace.join(script).canonicalize()?;
    let binary = binary.canonicalize()?;
    let artifact = out.join("observations.json");
    let check = crate::code::Check {
        id: scenario.into(),
        kind: "incident_drill".into(),
        program: python.into(),
        args: vec![
            script.to_string_lossy().into(),
            binary.to_string_lossy().into(),
            "--out".into(),
            artifact.to_string_lossy().into(),
        ],
        project_marker: None,
    };
    let execution = crate::code::run_observed(
        workspace,
        &check,
        "incident",
        scenario,
        std::time::Duration::from_secs(180),
        Some(&out.join("logs")),
        None,
    )?;
    if execution.status != "passed" {
        bail!("contained incident drill failed; inspect logs")
    };
    let observations: Value = serde_json::from_slice(&fs::read(&artifact)?)?;
    let report = json!({"scenario":scenario,"observations":observations,"observations_sha256":hash_file(&artifact)?,"script_sha256":hash_file(&script)?,"binary_sha256":hash_file(&binary)?,"execution":execution,"scope":"Actual contained fault injection on engineering fixtures; not permission to disrupt production systems."});
    fs::write(
        out.join("receipt.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}

pub fn catalog() -> Value {
    json!({"schema_version":1,"uses":[
{"id":"flight-recorder","name":"Software flight recorder","commands":["case-capture","case-rehearse","case-export"],"scope":"Selected source, checks, runtime/tool identities and receipts; not a full machine image."},
{"id":"comparison-lab","name":"Model and vendor comparison lab","commands":["lab-run","lab-compare"],"scope":"Executable prediction adapters, strict paired coverage, exact-match or MSE with group uncertainty; real provider identities and billing require additional evidence."},
{"id":"upgrade-rehearsal","name":"Upgrade rehearsal","commands":["case-capture","case-rehearse"],"scope":"Upgrade-purpose cases run the declared checks before and after an isolated candidate change."},
{"id":"source-impact","name":"Source correction impact","commands":["lineage-link","lineage-quarantine","lineage-check"],"scope":"Recorded dependency impact and opt-in promotion gate; not deletion or model unlearning."},
{"id":"reproducibility-exchange","name":"Reproducibility exchange","commands":["case-export","case-replay"],"scope":"Hash-anchored portable source/patch/check bundles. Toolchains/images must be provisioned separately."},
{"id":"bounded-maintenance","name":"Bounded maintenance","commands":["case-propose","case-rehearse","case-accept-remote","case-promote"],"scope":"External proposer contract, protected tests, semantic checks, optional required remote isolation, single-file promotion and rollback. Fixture proposer is deterministic, not a model."},
{"id":"compute-admission","name":"Compute admission broker","commands":["broker-quote","worker-enqueue --worker","worker-remote"],"scope":"Fresh supplied inventory, capability/resource/cost filters and identity-targeted claims; not provider provisioning, reservation or billing."},
{"id":"incident-rehearsal","name":"Contained incident rehearsal","commands":["incident-drill --scenario worker-loss","incident-drill --scenario remote-disconnect"],"scope":"Actual fault injection against contained engineering fixtures; not a live-production chaos agent."}
],"validation":"scripts/verify_uses.py; actual isolated remote case requires --image in the OCI CI job."})
}
