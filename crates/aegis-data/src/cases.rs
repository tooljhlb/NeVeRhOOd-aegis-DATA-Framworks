//! Portable, bounded change rehearsal: capture -> propose -> reproduce -> verify.
//! Commands require explicit host trust or a pinned OCI image. This is not a patch oracle.
use crate::{
    code::{self, Check},
    ingest::hash_file,
    isolation::Container,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
    time::Duration,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub schema_version: u32,
    pub id: String,
    pub purpose: String,
    pub files: Vec<String>,
    pub editable: Vec<String>,
    pub protected: Vec<String>,
    pub checks: Vec<Check>,
    pub baseline_exit_codes: BTreeMap<String, i32>,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub require_remote: bool,
    #[serde(default)]
    pub container: Option<Container>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    spec: Spec,
    inputs: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub edits: Vec<Edit>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub path: String,
    pub before: String,
    pub after: String,
    pub occurrences: usize,
}
pub(crate) fn relative(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 512
        && !p.contains('\\')
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn spec_valid(s: &Spec) -> Result<()> {
    if s.schema_version != 1
        || s.id.is_empty()
        || s.id.len() > 80
        || !s
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        || !["repair", "upgrade"].contains(&s.purpose.as_str())
        || s.files.is_empty()
        || s.files.len() > 10000
        || s.checks.is_empty()
        || s.checks.len() > 100
        || !(1..=3600).contains(&s.timeout_seconds)
        || s.timeout_seconds
            .saturating_mul(s.checks.len() as u64)
            .saturating_mul(2)
            > 86400
        || (s.purpose == "upgrade" && s.baseline_exit_codes.values().any(|code| *code != 0))
    {
        bail!("invalid case specification")
    }
    let files: BTreeSet<_> = s.files.iter().collect();
    let editable: BTreeSet<_> = s.editable.iter().collect();
    let protected: BTreeSet<_> = s.protected.iter().collect();
    if files.len() != s.files.len()
        || editable.len() != s.editable.len()
        || protected.len() != s.protected.len()
        || !editable.is_subset(&files)
        || !protected.is_subset(&files)
        || !editable.is_disjoint(&protected)
        || s.files.iter().any(|p| !relative(p))
    {
        bail!("invalid file policy")
    }
    let ids: BTreeSet<_> = s.checks.iter().map(|c| &c.id).collect();
    if ids.len() != s.checks.len()
        || ids.iter().any(|id| !relative(id) || id.contains('/'))
        || ids != s.baseline_exit_codes.keys().collect()
        || s.checks.iter().any(|c| c.program.is_empty())
    {
        bail!("check identities/expected exits required")
    }
    if let Some(c) = &s.container {
        c.validate()?
    }
    Ok(())
}
fn file(root: &Path, p: &str) -> Result<std::path::PathBuf> {
    if !relative(p) {
        bail!("invalid relative path")
    };
    let root = root.canonicalize()?;
    let mut cursor = root.clone();
    for component in Path::new(p).components() {
        cursor.push(component.as_os_str());
        if fs::symlink_metadata(&cursor)?.file_type().is_symlink() {
            bail!("symlink in selected path")
        }
    }
    let path = root.join(p);
    let real = path.canonicalize()?;
    if !real.starts_with(&root)
        || !real.is_file()
        || fs::symlink_metadata(&path)?.file_type().is_symlink()
    {
        bail!("file escaped root or symlink")
    };
    Ok(real)
}
fn selected(root: &Path, names: impl Iterator<Item = String>) -> Result<BTreeMap<String, String>> {
    let mut bytes = 0u64;
    let mut out = BTreeMap::new();
    for name in names {
        let path = file(root, &name)?;
        bytes += fs::metadata(&path)?.len();
        if bytes > 256 * 1024 * 1024 || out.len() >= 10000 {
            bail!("case input budget exceeded")
        };
        out.insert(name, hash_file(&path)?);
    }
    Ok(out)
}
fn copy(root: &Path, out: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    fs::create_dir_all(out)?;
    for (name, sha) in files {
        let src = file(root, name)?;
        if hash_file(&src)? != *sha {
            bail!("input changed during capture")
        };
        let dest = out.join(name);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(src, &dest)?;
        if hash_file(&dest)? != *sha {
            bail!("copy mismatch")
        }
    }
    Ok(())
}
fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    Ok(())
}
fn load(root: &Path) -> Result<Manifest> {
    let m: Manifest = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    spec_valid(&m.spec)?;
    if m.schema_version != 1
        || m.inputs.keys().cloned().collect::<BTreeSet<_>>()
            != m.spec.files.iter().cloned().collect()
        || selected(&root.join("bundle"), m.inputs.keys().cloned())? != m.inputs
    {
        bail!("case input identity mismatch")
    };
    Ok(m)
}
pub fn capture(source: &Path, spec: &Path, out: &Path) -> Result<Value> {
    let spec: Spec = serde_json::from_slice(&fs::read(spec)?)?;
    spec_valid(&spec)?;
    let inputs = selected(source, spec.files.iter().cloned())?;
    fs::create_dir(out)?;
    copy(source, &out.join("bundle"), &inputs)?;
    write(
        &out.join("manifest.json"),
        &Manifest {
            schema_version: 1,
            spec,
            inputs,
        },
    )?;
    Ok(
        json!({"case":out,"manifest_sha256":hash_file(&out.join("manifest.json"))?,"source_modified":false}),
    )
}
fn patch_valid(m: &Manifest, p: &Patch) -> Result<()> {
    if p.edits.is_empty() || p.edits.len() > 1000 {
        bail!("empty or oversized patch")
    };
    let mut paths = BTreeSet::new();
    for e in &p.edits {
        if !m.spec.editable.contains(&e.path)
            || m.spec.protected.contains(&e.path)
            || !paths.insert(&e.path)
            || e.before.is_empty()
            || e.before == e.after
            || e.before.len() + e.after.len() > 1024 * 1024
            || !(1..=100).contains(&e.occurrences)
        {
            bail!("patch violates edit policy")
        }
    }
    Ok(())
}
pub fn propose(root: &Path, generator: &Path, trusted: bool) -> Result<Value> {
    let m = load(root)?;
    let generator_sha256 = hash_file(generator)?;
    let c: Check = serde_json::from_slice(&fs::read(generator)?)?;
    let workspace = root.join("proposal");
    fs::create_dir(&workspace)?;
    copy(&root.join("bundle"), &workspace, &m.inputs)?;
    write(
        &workspace.join("aegis-request.json"),
        &json!({"schema_version":1,"case":m.spec.id,"editable":m.spec.editable,"protected":m.spec.protected,"inputs":m.inputs,"output_contract":"Write patch.json with edits:[{path,before,after,occurrences}]. No source promotion."}),
    )?;
    let result = execute(
        &workspace,
        &root.join("proposal-logs"),
        &c,
        &m.spec,
        trusted,
        "proposal",
    )?;
    if result.status != "passed" || hash_file(generator)? != generator_sha256 {
        bail!("proposal generator failed")
    };
    let path = file(&workspace, "patch.json")?;
    if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
        bail!("proposal too large")
    };
    let patch: Patch = serde_json::from_slice(&fs::read(&path)?)?;
    patch_valid(&m, &patch)?;
    let r = json!({"generator_sha256":generator_sha256,"execution":result,"patch_sha256":hash_file(&path)?,"source_modified":false,"scope":"Proposal supplied by executable adapter; model intelligence is not inferred."});
    write(&root.join("proposal-receipt.json"), &r)?;
    Ok(r)
}
fn execute(
    root: &Path,
    logs: &Path,
    c: &Check,
    s: &Spec,
    trusted: bool,
    name: &str,
) -> Result<code::CheckResult> {
    if !trusted && s.container.is_none() {
        bail!("case execution requires pinned container or --trusted-local")
    }
    let name = Container::name(&root.canonicalize()?.to_string_lossy(), name);
    let check = if let Some(container) = &s.container {
        Container::cleanup(&name)?;
        container.wrap(root, &name, c)?
    } else {
        c.clone()
    };
    let result = code::run_observed(
        root,
        &check,
        "case",
        &c.id,
        Duration::from_secs(s.timeout_seconds),
        Some(logs),
        None,
    );
    if s.container.is_some() {
        Container::cleanup(&name)?
    };
    result
}
pub fn rehearse(root: &Path, patch: &Path, trusted: bool) -> Result<Value> {
    let m = load(root)?;
    if fs::metadata(patch)?.len() > 16 * 1024 * 1024 {
        bail!("patch too large")
    };
    let p: Patch = serde_json::from_slice(&fs::read(patch)?)?;
    patch_valid(&m, &p)?;
    let baseline = root.join("baseline");
    let candidate = root.join("candidate");
    fs::create_dir(&baseline)?;
    fs::create_dir(&candidate)?;
    copy(&root.join("bundle"), &baseline, &m.inputs)?;
    copy(&root.join("bundle"), &candidate, &m.inputs)?;
    for e in &p.edits {
        let path = file(&candidate, &e.path)?;
        let original = fs::read_to_string(&path)?;
        if original.matches(&e.before).count() != e.occurrences {
            bail!("patch preimage occurrence mismatch")
        };
        fs::write(path, original.replace(&e.before, &e.after))?;
    }
    let candidate_inputs = selected(&candidate, m.inputs.keys().cloned())?;
    let tool_identities: BTreeMap<_, _> = m
        .spec
        .checks
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                crate::operator::executable_hash(
                    &candidate,
                    if m.spec.container.is_some() {
                        "docker"
                    } else {
                        &c.program
                    },
                ),
            )
        })
        .collect();
    let mut before = vec![];
    let mut after = vec![];
    let mut accepted = true;
    let mut reproduced = false;
    for c in &m.spec.checks {
        let r = execute(
            &baseline,
            &root.join("logs/baseline").join(&c.id),
            c,
            &m.spec,
            trusted,
            &format!("before-{}", c.id),
        )?;
        let expected = m.spec.baseline_exit_codes[&c.id];
        let observed =
            ["passed", "failed"].contains(&r.status.as_str()) && r.exit_code == Some(expected);
        accepted &= observed;
        reproduced |= observed && expected != 0;
        before.push(json!({"expected_exit":expected,"matched":observed,"execution":r}));
        let r = execute(
            &candidate,
            &root.join("logs/candidate").join(&c.id),
            c,
            &m.spec,
            trusted,
            &format!("after-{}", c.id),
        )?;
        accepted &= r.status == "passed" && r.exit_code == Some(0);
        after.push(r);
    }
    let inputs_stable = selected(&baseline, m.inputs.keys().cloned())? == m.inputs
        && selected(&candidate, m.inputs.keys().cloned())? == candidate_inputs;
    let tools_stable = m.spec.checks.iter().all(|c| {
        tool_identities[&c.id]
            == crate::operator::executable_hash(
                &candidate,
                if m.spec.container.is_some() {
                    "docker"
                } else {
                    &c.program
                },
            )
    });
    accepted &= tools_stable && inputs_stable && (m.spec.purpose == "upgrade" || reproduced);
    write(&root.join("patch.json"), &p)?;
    let report = json!({"schema_version":1,"case":m.spec.id,"purpose":m.spec.purpose,"accepted":accepted,"failure_reproduced":reproduced,"input_files_stable":inputs_stable,"tool_identities":tool_identities,"tools_stable":tools_stable,"runtime":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"binary_sha256":hash_file(&std::env::current_exe()?)?},"remote_required_for_promotion":m.spec.require_remote,"manifest_sha256":hash_file(&root.join("manifest.json"))?,"patch_sha256":hash_file(&root.join("patch.json"))?,"baseline":before,"candidate":after,"candidate_inputs":candidate_inputs,"scope":"Declared checks only; passing tests do not establish absence of all regressions."});
    write(&root.join("report.json"), &report)?;
    Ok(
        json!({"accepted":accepted,"report":report,"report_sha256":hash_file(&root.join("report.json"))?,"source_modified":false}),
    )
}
pub fn export(root: &Path, out: &Path) -> Result<Value> {
    let m = load(root)?;
    fs::create_dir(out)?;
    copy(&root.join("bundle"), &out.join("bundle"), &m.inputs)?;
    for name in ["manifest.json", "patch.json", "report.json"] {
        if root.join(name).is_file() {
            fs::copy(root.join(name), out.join(name))?;
        }
    }
    let files = crate::worker::inventory(out)?;
    write(
        &out.join("exchange.json"),
        &json!({"schema_version":1,"files":files,"scope":"Portable source/patch/check contract; toolchains and OCI images must be provisioned separately."}),
    )?;
    Ok(json!({"package":out,"exchange_sha256":hash_file(&out.join("exchange.json"))?}))
}
pub fn replay(package: &Path, expected: &str, out: &Path, trusted: bool) -> Result<Value> {
    if hash_file(&package.join("exchange.json"))? != expected {
        bail!("exchange identity mismatch")
    };
    let exchange: Value = serde_json::from_slice(&fs::read(package.join("exchange.json"))?)?;
    let files: BTreeMap<String, String> = serde_json::from_value(exchange["files"].clone())?;
    let actual = crate::worker::inventory(package)?;
    let actual: BTreeMap<_, _> = actual
        .into_iter()
        .filter(|(p, _)| p != "exchange.json")
        .collect();
    if files != actual {
        bail!("exchange membership/hash mismatch")
    };
    let m = load(package)?;
    fs::create_dir(out)?;
    copy(&package.join("bundle"), &out.join("bundle"), &m.inputs)?;
    write(&out.join("manifest.json"), &m)?;
    rehearse(out, &package.join("patch.json"), trusted)
}
pub fn promote(
    root: &Path,
    source: &Path,
    expected_report: &str,
    rollback: bool,
    remote: Option<(&str, &str)>,
    lineage: Option<&str>,
) -> Result<Value> {
    let m = load(root)?;
    if hash_file(&root.join("report.json"))? != expected_report {
        bail!("reviewed report identity changed")
    };
    let report: Value = serde_json::from_slice(&fs::read(root.join("report.json"))?)?;
    if report["accepted"] != true
        || report["manifest_sha256"] != hash_file(&root.join("manifest.json"))?
        || report["patch_sha256"] != hash_file(&root.join("patch.json"))?
    {
        bail!("case not accepted or changed")
    };
    if !rollback {
        if m.spec.require_remote && remote.is_none() {
            bail!("case policy requires remote isolated verification")
        }
        if let Some((db, job)) = remote {
            accept_remote(root, db, job)?;
        }
        if let Some(db) = lineage {
            crate::uses::allow(db, expected_report)?;
        }
    }
    let p: Patch = serde_json::from_slice(&fs::read(root.join("patch.json"))?)?;
    patch_valid(&m, &p)?;
    let candidate: BTreeMap<String, String> =
        serde_json::from_value(report["candidate_inputs"].clone())?;
    if selected(&root.join("candidate"), m.inputs.keys().cloned())? != candidate {
        bail!("candidate source changed")
    };
    let expected = if rollback { &candidate } else { &m.inputs };
    if selected(source, m.inputs.keys().cloned())? != *expected {
        bail!("project changed since reviewed experiment")
    };
    if p.edits.len() != 1 {
        bail!("automatic promotion is limited to one file; review multi-file changes separately")
    }
    let action = if rollback { "rollback" } else { "promote" };
    let journal = root.join(format!("promotion-{}-{}.json", action, std::process::id()));
    write(
        &journal,
        &json!({"state":"pending","action":action,"source":source,"report_sha256":expected_report,"original_inputs":m.inputs,"candidate_inputs":candidate}),
    )?;
    let mut changed = vec![];
    for e in &p.edits {
        let dst = file(source, &e.path)?;
        let replacement = if rollback {
            root.join("bundle").join(&e.path)
        } else {
            root.join("candidate").join(&e.path)
        };
        let tmp = dst.with_extension(format!("aegis-{}.tmp", std::process::id()));
        if tmp.exists() {
            bail!("promotion temp already exists")
        };
        let permissions = fs::metadata(&dst)?.permissions();
        fs::copy(replacement, &tmp)?;
        fs::set_permissions(&tmp, permissions)?;
        fs::File::open(&tmp)?.sync_all()?;
        if hash_file(&dst)? != expected[&e.path] {
            fs::remove_file(&tmp)?;
            bail!("source changed during promotion")
        }
        fs::rename(tmp, dst)?;
        changed.push(e.path.clone());
    }
    let result = json!({"state":"completed","action":action,"changed":changed,"report_sha256":expected_report,"journal":journal,"scope":"Single-file atomic replacement after full preflight. Original bundle retained for rollback; pending journal supports manual crash reconciliation."});
    let sealed = journal.with_extension("complete.tmp");
    write(&sealed, &result)?;
    fs::rename(&sealed, &journal)?;
    Ok(result)
}

pub fn accept_remote(root: &Path, db: &str, job: &str) -> Result<Value> {
    let m = load(root)?;
    let report: Value = serde_json::from_slice(&fs::read(root.join("report.json"))?)?;
    if report["accepted"] != true
        || report["manifest_sha256"] != hash_file(&root.join("manifest.json"))?
        || report["patch_sha256"] != hash_file(&root.join("patch.json"))?
    {
        bail!("local evidence changed or rejected")
    }
    let result = crate::remote::verify_return(db, job)?;
    let state = crate::ledger::Ledger::open(db)?.inspect(job)?;
    let payload: crate::worker::Payload = serde_json::from_str(
        state["payload"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing job payload"))?,
    )?;
    let candidate: BTreeMap<String, String> =
        serde_json::from_value(report["candidate_inputs"].clone())?;
    if payload.files != candidate || payload.plan.stages.len() != m.spec.checks.len() {
        bail!("remote source/check membership differs from reviewed candidate")
    }
    for check in &m.spec.checks {
        let stage = payload
            .plan
            .stages
            .iter()
            .find(|s| s.id == check.id)
            .ok_or_else(|| anyhow::anyhow!("remote check missing"))?;
        if stage.program != check.program
            || stage.args != check.args
            || stage.container.is_none()
            || stage.outputs.is_empty()
        {
            bail!("remote check identity or isolation differs")
        }
    }
    Ok(
        json!({"accepted":true,"case_report_sha256":hash_file(&root.join("report.json"))?,"remote_plan_sha256":crate::ingest::hash(&serde_json::to_vec(&payload.plan)?),"remote_images":payload.plan.stages.iter().map(|s|json!({"stage":s.id,"container":s.container})).collect::<Vec<_>>(),"remote":result,"scope":"Local checks plus pinned-source/check remote transport with required OCI; worker remains trusted, not hardware-attested."}),
    )
}
