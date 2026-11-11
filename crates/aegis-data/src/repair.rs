//! Isolated formatter/lint proposals with explicit apply/rollback and hash checks.
use crate::{
    code::{self, Check},
    ingest::hash_file,
};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path, time::Duration};
pub fn propose(source: &Path, out: &Path, tool: &str) -> Result<Value> {
    let source = source.canonicalize()?;
    if !source.is_file() {
        bail!("source must be a file")
    };
    if !["ruff-format", "ruff-fix", "clang-format"].contains(&tool) {
        bail!("unknown repair tool")
    }
    fs::create_dir(out)?;
    let out = out.canonicalize()?;
    let original = out.join(".aegis-original");
    fs::copy(&source, &original)?;
    let mut candidate = out.join("candidate");
    if let Some(extension) = source.extension() {
        candidate.set_extension(extension);
    }
    fs::copy(&source, &candidate)?;
    let candidate_arg = candidate.to_string_lossy().into_owned();
    let (program, args) = match tool {
        "ruff-format" => ("ruff", vec!["format", "--isolated", &candidate_arg]),
        "ruff-fix" => ("ruff", vec!["check", "--isolated", "--fix", &candidate_arg]),
        _ => ("clang-format", vec!["-i", &candidate_arg]),
    };
    let c = Check {
        id: "repair-candidate".into(),
        kind: "format".into(),
        program: program.into(),
        args: args.into_iter().map(String::from).collect(),
        project_marker: None,
    };
    let executed = code::run(&out, &c, "repair", "candidate", Duration::from_secs(60))?;
    let verify = if tool.starts_with("ruff") {
        Check {
            id: "candidate-check".into(),
            kind: "lint".into(),
            program: "ruff".into(),
            args: vec!["check".into(), "--isolated".into(), candidate_arg],
            project_marker: None,
        }
    } else {
        Check {
            id: "candidate-check".into(),
            kind: "format".into(),
            program: "clang-format".into(),
            args: vec!["--dry-run".into(), "--Werror".into(), candidate_arg],
            project_marker: None,
        }
    };
    let checked = code::run(
        &out,
        &verify,
        "repair",
        "candidate",
        Duration::from_secs(60),
    )?;
    let receipt = json!({"schema_version":1,"source":source,"original":original,"candidate":candidate,"original_sha256":hash_file(&original)?,"candidate_sha256":hash_file(&candidate)?,"tool":tool,"execution":executed,"check":checked,"status":if executed.status=="passed"&&checked.status=="passed"{"candidate_lint_passed"}else{"rejected"},"semantic_tests":"not_run; require project tests before automatic promotion","source_modified":false});
    fs::write(
        out.join("proposal.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(receipt)
}
pub fn apply(source: &Path, receipt: &Path, rollback: bool) -> Result<Value> {
    let r: Value = serde_json::from_slice(&fs::read(receipt)?)?;
    let source = source.canonicalize()?;
    if r["status"] != "candidate_lint_passed"
        || r["source"].as_str() != Some(source.to_string_lossy().as_ref())
    {
        bail!("proposal does not authorize this source")
    }
    let (expected, replacement, replacement_hash) = if rollback {
        (
            &r["candidate_sha256"],
            &r["original"],
            &r["original_sha256"],
        )
    } else {
        (
            &r["original_sha256"],
            &r["candidate"],
            &r["candidate_sha256"],
        )
    };
    if hash_file(&source)? != expected.as_str().unwrap_or("") {
        bail!("source changed since proposal; refusing replacement")
    }
    let replacement = Path::new(
        replacement
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing replacement"))?,
    );
    if hash_file(replacement)? != replacement_hash.as_str().unwrap_or("") {
        bail!("replacement hash mismatch")
    }
    let tmp = source.with_file_name(format!(".aegis-repair-{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp)?;
    let bytes = fs::read(replacement)?;
    f.write_all(&bytes)?;
    f.set_permissions(fs::metadata(&source)?.permissions())?;
    f.sync_all()?;
    drop(f);
    if hash_file(&source)? != expected.as_str().unwrap_or("") {
        let _ = fs::remove_file(tmp);
        bail!("source changed during replacement")
    }
    fs::rename(&tmp, &source)?;
    let result = json!({"source":source,"action":if rollback{"rollback"}else{"apply"},"output_sha256":hash_file(&source)?,"proposal_sha256":hash_file(receipt)?});
    let name = if rollback {
        "rollback.json"
    } else {
        "applied.json"
    };
    fs::write(
        receipt.parent().unwrap().join(name),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
