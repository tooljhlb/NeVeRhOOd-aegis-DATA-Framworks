//! Installed-runtime project checks and bounded evidence export.
use crate::{
    ingest::hash_file,
    operator::{self, Plan},
};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn doctor(plan: &Plan) -> Result<Value> {
    plan.validate()?;
    let root = plan.workspace.canonicalize()?;
    let tools: Vec<_> = plan
        .stages
        .iter()
        .map(|s| {
            let program = if s.container.is_some() {
                "docker"
            } else {
                &s.program
            };
            let hash = operator::executable_hash(&root, program);
            json!({"stage":s.id,"program":program,"available":hash.is_some(),"sha256":hash})
        })
        .collect();
    Ok(
        json!({"ready":tools.iter().all(|t|t["available"]==true),"workspace":root,"tools":tools,
        "scope":"executable presence and plan validation; not tool behavior, Docker daemon or GPU certification"}),
    )
}
fn copy(
    root: &Path,
    relative: &Path,
    dest: &Path,
    files: &mut BTreeMap<String, String>,
    total: &mut u64,
) -> Result<()> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("invalid export path")
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("export refuses symlinks")
        }
    }
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        bail!("export expects regular files")
    }
    *total = total
        .checked_add(metadata.len())
        .ok_or_else(|| anyhow::anyhow!("export size overflow"))?;
    if *total > 256 * 1024 * 1024 || files.len() >= 10000 {
        bail!("export exceeds bounded package limits")
    }
    let target = dest.join(relative);
    fs::create_dir_all(target.parent().unwrap())?;
    let before = hash_file(&path)?;
    fs::copy(&path, &target)?;
    if hash_file(&path)? != before || hash_file(&target)? != before {
        bail!("file changed during export")
    }
    files.insert(relative.to_string_lossy().into_owned(), before);
    Ok(())
}
pub fn export(db: &str, run: &str, out: &Path) -> Result<Value> {
    let state = operator::status(db, run)?;
    if state["status"] != "completed" {
        bail!("only completed runs may be exported")
    }
    let plan: Plan = serde_json::from_value(state["plan"].clone())?;
    plan.validate()?;
    let root = plan.workspace.canonicalize()?;
    let graph = operator::open(db)?;
    graph.verify()?;
    let plan_hash: String = graph.db.query_row(
        "SELECT plan_hash FROM operator_runs WHERE id=?1",
        [run],
        |r| r.get(0),
    )?;
    if crate::ingest::hash(serde_json::to_string(&plan)?.as_bytes()) != plan_hash {
        bail!("stored plan identity changed")
    }

    let (event_count, event_bytes): (u64, u64) = graph.db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(LENGTH(payload)),0) FROM fabric_events WHERE run_id=?1",
        [run],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if event_count > 1000 || event_bytes > 16 * 1024 * 1024 {
        bail!("run event export exceeds bounded limit")
    }
    let logs = operator::log_root(db, run)?.canonicalize()?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let destination = parent.join(
        out.file_name()
            .ok_or_else(|| anyhow::anyhow!("missing export directory name"))?,
    );
    if destination.starts_with(&logs) {
        bail!("export cannot be inside source logs")
    }

    let events = graph.events(run, 0)?;
    if events.len() as u64 != event_count {
        bail!("run events changed during export")
    }
    // Validate all declared inputs, outputs and executable identities against receipts.
    let mut artifacts = BTreeMap::new();
    for stage in &plan.stages {
        let row = state["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == stage.id)
            .ok_or_else(|| anyhow::anyhow!("missing stage"))?;
        let recorded = events
            .iter()
            .rev()
            .find(|e| e["kind"] == "stage_finished" && e["payload"]["stage"] == stage.id)
            .ok_or_else(|| anyhow::anyhow!("missing recorded execution receipt"))?;
        if recorded["payload"] != row["receipt"] {
            bail!("stage receipt differs from verified event ledger")
        }
        let program = if stage.container.is_some() {
            "docker"
        } else {
            &stage.program
        };
        if row["status"] != "passed"
            || serde_json::to_value(operator::executable_hash(&root, program))?
                != row["receipt"]["program_sha256"]
        {
            bail!("execution identity changed")
        }
        for kind in ["inputs", "outputs"] {
            let expected = row["receipt"][kind]
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("missing artifact identities"))?;
            for (path, hash) in expected {
                if hash_file(&root.join(path))? != hash.as_str().unwrap_or("") {
                    bail!("declared artifact changed: {path}")
                }
                artifacts.insert(path.clone(), hash.clone());
            }
        }
    }
    fs::create_dir(out)?;
    let result = (|| -> Result<Value> {
        let mut files = BTreeMap::new();
        let mut total = 0;
        let payload = out.join("workspace");
        fs::create_dir(&payload)?;
        for (path, expected) in artifacts {
            copy(&root, Path::new(&path), &payload, &mut files, &mut total)?;
            if files[&path] != expected.as_str().unwrap_or("") {
                bail!("artifact changed before copy")
            }
        }
        let workspace_files = files;
        let mut log_files = BTreeMap::new();

        let mut queue = vec![PathBuf::new()];
        let mut entries = 0;
        while let Some(relative) = queue.pop() {
            for entry in fs::read_dir(logs.join(&relative))? {
                let entry = entry?;
                entries += 1;
                if entries > 20000 {
                    bail!("log membership limit exceeded")
                }
                let path = relative.join(entry.file_name());
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    bail!("log export refuses symlinks")
                }
                if kind.is_dir() {
                    queue.push(path);
                } else {
                    copy(&logs, &path, &out.join("logs"), &mut log_files, &mut total)?;
                    if workspace_files.len() + log_files.len() > 10000 {
                        bail!("export file count exceeded")
                    }
                }
            }
        }
        fs::write(out.join("run.json"), serde_json::to_vec_pretty(&state)?)?;
        fs::write(out.join("events.json"), serde_json::to_vec_pretty(&events)?)?;
        let manifest = json!({"schema_version":1,"run_id":run,"status":"completed_declared_checks",
            "workspace_files":workspace_files,"log_files":log_files,"run_sha256":hash_file(&out.join("run.json"))?,
            "events_sha256":hash_file(&out.join("events.json"))?,"scope":"declared checks and byte identities; not general software correctness"});
        fs::write(
            out.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(
            json!({"export":out,"manifest_sha256":hash_file(&out.join("manifest.json"))?,"run_id":run}),
        )
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(out);
    }
    result
}

pub fn verify_export(root: &Path, expected_manifest: &str) -> Result<Value> {
    let root = root.canonicalize()?;
    let manifest_path = root.join("manifest.json");
    if fs::symlink_metadata(&manifest_path)?
        .file_type()
        .is_symlink()
        || fs::metadata(&manifest_path)?.len() > 4 * 1024 * 1024
        || hash_file(&manifest_path)? != expected_manifest
    {
        bail!("export manifest anchor mismatch")
    }
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    if manifest["schema_version"] != 1 || manifest["status"] != "completed_declared_checks" {
        bail!("unsupported export manifest")
    }
    let mut expected = BTreeMap::new();
    expected.insert("manifest.json".to_string(), expected_manifest.to_string());
    for (name, key) in [("run.json", "run_sha256"), ("events.json", "events_sha256")] {
        expected.insert(
            name.into(),
            manifest[key]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("missing metadata hash"))?
                .to_string(),
        );
    }
    for (prefix, key) in [("workspace", "workspace_files"), ("logs", "log_files")] {
        for (name, hash) in manifest[key]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("missing file map"))?
        {
            let relative = Path::new(name);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                bail!("invalid package member")
            }
            expected.insert(
                format!("{prefix}/{name}"),
                hash.as_str()
                    .ok_or_else(|| anyhow::anyhow!("invalid file hash"))?
                    .to_string(),
            );
        }
    }
    let mut queue = vec![PathBuf::new()];
    let mut count = 0;
    let mut size: u64 = 0;
    while let Some(relative) = queue.pop() {
        for entry in fs::read_dir(root.join(relative))? {
            let entry = entry?;
            let path = entry.path();
            count += 1;
            if count > 20000 {
                bail!("package membership limit exceeded")
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!("package symlink rejected")
            }
            let relative = path.strip_prefix(&root)?;
            if metadata.is_dir() {
                queue.push(relative.to_path_buf());
                continue;
            }
            if !metadata.is_file() {
                bail!("package special file rejected")
            }
            size = size
                .checked_add(metadata.len())
                .ok_or_else(|| anyhow::anyhow!("package size overflow"))?;
            if size > 280 * 1024 * 1024 {
                bail!("package too large")
            }
            let key = relative.to_string_lossy();
            let hash = expected
                .remove(key.as_ref())
                .ok_or_else(|| anyhow::anyhow!("unexpected package member"))?;
            if hash_file(&path)? != hash {
                bail!("package hash mismatch")
            }
        }
    }
    if !expected.is_empty() {
        bail!("missing package member")
    }
    Ok(
        json!({"verified":true,"manifest_sha256":expected_manifest,"scope":"anchored file membership and byte integrity; not independent execution attestation"}),
    )
}
