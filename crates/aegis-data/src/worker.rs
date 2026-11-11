//! Durable shared-volume workers: immutable bundle, fenced epoch, heartbeat, sealed return.
//! The shared SQLite database/spool are trusted controller storage, not an internet protocol.
use crate::{ingest::hash_file, ledger::Ledger, operator::Plan};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Payload {
    #[serde(default)]
    pub(crate) required_worker: Option<String>,
    pub(crate) plan: Plan,
    pub(crate) bundle: PathBuf,
    pub(crate) files: BTreeMap<String, String>,
}
fn now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}
fn valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && id != "."
        && id != ".."
}
fn spool(db: &str) -> Result<PathBuf> {
    let path = PathBuf::from(format!("{db}.workers"));
    fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}
pub(crate) fn inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    fn visit(
        root: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, String>,
        bytes: &mut u64,
    ) -> Result<()> {
        for item in fs::read_dir(dir)? {
            let p = item?.path();
            let meta = fs::symlink_metadata(&p)?;
            if meta.file_type().is_symlink() {
                bail!("worker bundles cannot contain symlinks")
            }
            if meta.is_dir() {
                visit(root, &p, out, bytes)?
            } else if meta.is_file() {
                *bytes = bytes.checked_add(meta.len()).context("bundle overflow")?;
                if *bytes > 256 * 1024 * 1024 || out.len() >= 10000 {
                    bail!("bundle limit: 256MiB/10000 files")
                };
                let rel = p
                    .strip_prefix(root)?
                    .to_str()
                    .context("UTF-8 path required")?
                    .to_string();
                out.insert(rel, hash_file(&p)?);
            } else {
                bail!("special files forbidden")
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out, &mut 0)?;
    Ok(out)
}
fn copy(root: &Path, out: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    fs::create_dir_all(out)?;
    for (name, sha) in files {
        let source = root.join(name);
        let dest = out.join(name);
        if fs::symlink_metadata(&source)?.file_type().is_symlink() || hash_file(&source)? != *sha {
            bail!("bundle identity changed")
        };
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(&source, &dest)?;
        if hash_file(&dest)? != *sha {
            bail!("bundle copy mismatch")
        }
    }
    if inventory(out)? != *files {
        bail!("bundle membership changed")
    };
    Ok(())
}
pub fn enqueue(db: &str, plan_path: &Path, bundle: &Path) -> Result<Value> {
    enqueue_for(db, plan_path, bundle, None)
}
pub fn enqueue_for(
    db: &str,
    plan_path: &Path,
    bundle: &Path,
    required_worker: Option<String>,
) -> Result<Value> {
    if required_worker.as_ref().is_some_and(|s| !valid(s)) {
        bail!("invalid required worker")
    }

    let mut plan: Plan = serde_json::from_slice(&fs::read(plan_path)?)?;
    plan.validate()?;
    if !valid(&plan.run_id) {
        bail!("invalid worker job ID")
    };
    // Workspace is a worker-owned copy; the submitted host path has no authority.
    plan.workspace = PathBuf::from(".");
    let source = bundle.canonicalize()?;
    let files = inventory(&source)?;
    let destination = spool(db)?.join(&plan.run_id).join("bundle");
    if destination.exists() {
        if inventory(&destination)? != files {
            bail!("job bundle already exists with different identity")
        }
    } else {
        copy(&source, &destination, &files)?
    }
    let id = plan.run_id.clone();
    let payload = serde_json::to_string(&Payload {
        required_worker,
        plan,
        bundle: destination,
        files,
    })?;
    Ok(json!({"inserted":Ledger::open(db)?.enqueue(&id,&payload)?,"id":id}))
}
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub fn work(db: &str, worker: &str, ttl: i64, trusted: bool) -> Result<Value> {
    if !trusted || !valid(worker) || !(3..=300).contains(&ttl) {
        bail!("trusted worker ID and ttl 3..300 required")
    }
    let mut ledger = Ledger::open(db)?;
    let Some(lease) = ledger.claim(worker, now()?, ttl)? else {
        return Ok(json!({"state":"idle"}));
    };
    let result = (|| -> Result<Value> {
        let job = ledger.inspect(&lease.id)?;
        let mut payload: Payload = serde_json::from_str(job["payload"].as_str().unwrap())?;
        if inventory(&payload.bundle)? != payload.files {
            bail!("queued bundle changed")
        }
        let attempt = spool(db)?
            .join(&lease.id)
            .join(format!("epoch-{}", lease.epoch));
        // Reconcile containers from lost epochs before granting this attempt work.
        for old in 1..lease.epoch {
            let previous = spool(db)?
                .join(&lease.id)
                .join(format!("epoch-{old}/workspace"));
            for stage in &payload.plan.stages {
                if stage.container.is_some() {
                    let name = crate::isolation::Container::name(
                        &format!("{}:{}-e{}", previous.display(), lease.id, old),
                        &stage.id,
                    );
                    crate::isolation::Container::cleanup(&name)?;
                }
            }
        }
        let workspace = attempt.join("workspace");
        copy(&payload.bundle, &workspace, &payload.files)?;
        payload.plan.workspace = workspace.clone();
        payload.plan.run_id = format!("{}-e{}", lease.id, lease.epoch);
        let plan_file = attempt.join("plan.json");
        fs::write(&plan_file, serde_json::to_vec_pretty(&payload.plan)?)?;
        let operator_db = attempt.join("operator.db");
        let mut command = Command::new(std::env::current_exe()?);
        command.env_clear();
        for key in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .args([
                "operator-run",
                operator_db.to_str().context("UTF8 DB")?,
                plan_file.to_str().unwrap(),
                "--trusted-local",
            ])
            .stdout(Stdio::from(fs::File::create(attempt.join("stdout.log"))?))
            .stderr(Stdio::from(fs::File::create(attempt.join("stderr.log"))?));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
            #[cfg(target_os = "linux")]
            {
                let parent = std::process::id();
                unsafe {
                    command.pre_exec(move || {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0
                            || libc::getppid() != parent as i32
                        {
                            return Err(std::io::Error::other("worker parent lost"));
                        }
                        Ok(())
                    });
                }
            }
        }
        let mut child = command.spawn()?;
        let started = Instant::now();
        let mut heartbeat = Instant::now();
        loop {
            // Renewal doubles as cancellation/stale-fence detection, even at completion.
            if heartbeat.elapsed() >= Duration::from_millis((ttl as u64) * 250) {
                if let Err(e) = ledger.renew(&lease, now()?, ttl) {
                    stop(&mut child);
                    return Err(e);
                }
                heartbeat = Instant::now();
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("worker operator failed; inspect epoch logs")
                };
                break;
            }
            if started.elapsed() > Duration::from_secs(payload.plan.max_wall_seconds + 10) {
                stop(&mut child);
                bail!("worker deadline exceeded")
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        ledger.renew(&lease, now()?, ttl)?;
        let state = crate::operator::status(operator_db.to_str().unwrap(), &payload.plan.run_id)?;
        if state["status"] != "completed" {
            bail!("worker returned incomplete run")
        }
        crate::operator::open(operator_db.to_str().unwrap())?.verify()?;
        let mut outputs = BTreeMap::new();
        for stage in &payload.plan.stages {
            for path in &stage.outputs {
                let target = workspace.join(path).canonicalize()?;
                if !target.starts_with(&workspace) {
                    bail!("worker output escapes workspace")
                }
                outputs.insert(path.clone(), hash_file(&target)?);
            }
        }
        let sealed = json!({"schema_version":1,"job_id":lease.id,"worker":worker,"epoch":lease.epoch,"workspace":workspace,"inputs":payload.files,"outputs":outputs,"operator_db":operator_db,"operator_run_id":payload.plan.run_id});
        let receipt = attempt.join("return.json");
        fs::write(&receipt, serde_json::to_vec_pretty(&sealed)?)?;
        let result = json!({"receipt":receipt,"sha256":hash_file(&receipt)?});
        ledger.complete(&lease, now()?, &serde_json::to_string(&result)?)?;
        Ok(json!({"state":"done","id":lease.id,"epoch":lease.epoch,"result":result}))
    })();
    if let Err(e) = &result {
        let payload: Payload =
            serde_json::from_str(ledger.inspect(&lease.id)?["payload"].as_str().unwrap())?;
        let workspace = spool(db)?
            .join(&lease.id)
            .join(format!("epoch-{}/workspace", lease.epoch));
        for stage in &payload.plan.stages {
            if stage.container.is_some() {
                let name = crate::isolation::Container::name(
                    &format!("{}:{}-e{}", workspace.display(), lease.id, lease.epoch),
                    &stage.id,
                );
                crate::isolation::Container::cleanup(&name)?;
            }
        }
        let _ = ledger.fail(&lease, now()?, &e.to_string());
    }
    result
}
pub fn verify(db: &str, id: &str) -> Result<Value> {
    let state = Ledger::open(db)?.inspect(id)?;
    if state["state"] != "done" {
        bail!("job has no accepted result")
    }
    let result: Value = serde_json::from_str(state["result"].as_str().unwrap())?;
    let path = Path::new(result["receipt"].as_str().unwrap());
    if hash_file(path)? != result["sha256"].as_str().unwrap() {
        bail!("worker receipt changed")
    }
    let receipt: Value = serde_json::from_slice(&fs::read(path)?)?;
    if receipt["epoch"] != state["epoch"]
        || receipt["job_id"] != id
        || receipt["worker"] != state["worker"]
    {
        bail!("stale worker return")
    }
    let root = Path::new(receipt["workspace"].as_str().unwrap()).canonicalize()?;
    for (name, sha) in receipt["outputs"].as_object().context("outputs")? {
        let output = root.join(name).canonicalize()?;
        if !output.starts_with(&root) || hash_file(&output)? != sha.as_str().unwrap() {
            bail!("returned output changed")
        }
    }
    crate::operator::open(receipt["operator_db"].as_str().unwrap())?.verify()?;
    Ok(
        json!({"verified":true,"id":id,"epoch":state["epoch"],"receipt_sha256":result["sha256"],"scope":"trusted shared-volume worker; no remote attestation"}),
    )
}
