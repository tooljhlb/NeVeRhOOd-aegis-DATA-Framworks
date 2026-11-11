//! Durable local operator: bounded DAG execution, policy, receipts, controls.
//! Commands execute only in explicitly trusted workspaces; this is not a sandbox.
use crate::{
    code::{self, Check},
    graph::GraphStore,
    ingest::hash_file,
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
fn one() -> usize {
    1
}
fn timeout() -> u64 {
    120
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub run_id: String,
    pub workspace: PathBuf,
    pub mode: String,
    pub max_slots: usize,
    pub max_wall_seconds: u64,
    pub allowed_operations: Vec<String>,
    pub stages: Vec<Stage>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub id: String,
    pub operation: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default = "one")]
    pub slots: usize,
    #[serde(default = "one")]
    pub max_attempts: usize,
    #[serde(default)]
    pub retry_exit_codes: Vec<i32>,
    #[serde(default)]
    pub resume_args: Vec<String>,
    #[serde(default)]
    pub container: Option<crate::isolation::Container>,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub metric_log: Option<String>,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 96
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn relative(p: &str) -> bool {
    !p.is_empty()
        && !Path::new(p).is_absolute()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
impl Plan {
    pub fn validate(&self) -> Result<()> {
        if !identifier(&self.run_id)
            || !["observe", "assist", "bounded", "autonomous_workspace"]
                .contains(&self.mode.as_str())
            || !(1..=16).contains(&self.max_slots)
            || !(1..=86400).contains(&self.max_wall_seconds)
            || self.stages.is_empty()
            || self.stages.len() > 1000
        {
            bail!("invalid operator plan")
        }
        let mut ids = BTreeSet::new();
        for s in &self.stages {
            if let Some(container) = &s.container {
                container.validate()?;
            }
            if !identifier(&s.id)
                || !ids.insert(&s.id)
                || s.program.is_empty()
                || s.slots == 0
                || s.slots > self.max_slots
                || !(1..=5).contains(&s.max_attempts)
                || !(1..=3600).contains(&s.timeout_seconds)
                || s.inputs.iter().chain(&s.outputs).any(|p| !relative(p))
                || s.metric_log.as_ref().is_some_and(|p| !relative(p))
            {
                bail!("invalid stage or artifact path")
            }
        }
        let mut visited = BTreeSet::new();
        loop {
            let before = visited.len();
            for s in &self.stages {
                if s.depends_on.iter().any(|id| !ids.contains(id)) {
                    bail!("unknown dependency")
                };
                if s.depends_on.iter().all(|id| visited.contains(id)) {
                    visited.insert(s.id.clone());
                }
            }
            if visited.len() == self.stages.len() {
                break;
            }
            if visited.len() == before {
                bail!("dependency cycle")
            }
        }
        Ok(())
    }
}
pub fn open(db: &str) -> Result<GraphStore> {
    let g = GraphStore::open(db)?;
    g.db.execute_batch("CREATE TABLE IF NOT EXISTS operator_runs(id TEXT PRIMARY KEY,plan_hash TEXT NOT NULL,plan TEXT NOT NULL,status TEXT NOT NULL,control TEXT NOT NULL DEFAULT 'run'); CREATE TABLE IF NOT EXISTS operator_stages(run_id TEXT NOT NULL,id TEXT NOT NULL,status TEXT NOT NULL,attempts INTEGER NOT NULL,receipt TEXT,PRIMARY KEY(run_id,id));")?;
    Ok(g)
}
pub fn status(db: &str, run: &str) -> Result<Value> {
    let g = open(db)?;
    let (state, control, plan): (String, String, String) = g.db.query_row(
        "SELECT status,control,plan FROM operator_runs WHERE id=?1",
        [run],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut q = g.db.prepare(
        "SELECT id,status,attempts,receipt FROM operator_stages WHERE run_id=?1 ORDER BY id",
    )?;
    let mut stages = Vec::new();
    for row in q.query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })? {
        let (id, s, a, receipt) = row?;
        stages.push(json!({"id":id,"status":s,"attempts":a,"receipt":receipt.map(|x|serde_json::from_str::<Value>(&x)).transpose()?}));
    }
    Ok(
        json!({"run_id":run,"status":state,"control":control,"plan":serde_json::from_str::<Value>(&plan)?,"stages":stages}),
    )
}
pub fn control(db: &str, run: &str, action: &str, actor: &str) -> Result<()> {
    if !["pause", "cancel", "run"].contains(&action) {
        bail!("unknown control")
    };
    let mut g = open(db)?;
    let n = g.db.execute(
        "UPDATE operator_runs SET control=?2 WHERE id=?1",
        params![run, action],
    )?;
    if n != 1 {
        bail!("unknown run")
    };
    g.record(
        run,
        "control",
        json!({"action":action,"actor":actor,"pause_semantics":"stage_boundary"}),
    )?;
    Ok(())
}
fn artifacts(root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for p in paths {
        let actual = root.join(p).canonicalize()?;
        if !actual.starts_with(root) || !actual.is_file() {
            bail!("artifact escapes workspace or is not a file: {p}")
        }
        out.insert(p.clone(), hash_file(&actual)?);
    }
    Ok(out)
}
pub fn log_root(db: &str, run: &str) -> Result<PathBuf> {
    if !identifier(run) {
        bail!("invalid run id")
    }
    Ok(PathBuf::from(format!("{db}.artifacts")).join(run))
}
fn rejection(g: &mut GraphStore, run: &str, reason: &str, stage: Option<&str>) -> Result<()> {
    g.record(run,"rejection",json!({"reason":reason,"stage":stage,"rule":"operator_policy_or_verification","choices":["inspect_logs","correct_input_or_plan","create_new_run"],"override":"new explicit plan required; failed evidence cannot be promoted"}))?;
    g.record(
        run,
        "notification",
        json!({"severity":"error","message":reason,"stage":stage}),
    )?;
    Ok(())
}
pub(crate) fn executable_hash(root: &Path, program: &str) -> Option<String> {
    if program.contains(std::path::MAIN_SEPARATOR) {
        return hash_file(&root.join(program)).ok();
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|p| p.join(program))
                .find(|p| p.is_file())
        })
        .and_then(|p| hash_file(&p).ok())
}
fn execute_stage(db: &str, plan: &Plan, stage: &Stage, cancel: Arc<AtomicBool>) -> Result<bool> {
    let root = plan.workspace.canonicalize()?;
    let mut g = open(db)?;
    let inputs = artifacts(&root, &stage.inputs)?;
    let program_sha256 = executable_hash(
        &root,
        if stage.container.is_some() {
            "docker"
        } else {
            &stage.program
        },
    );
    for (path, sha) in &inputs {
        let artifact = format!("artifact:{sha}");
        g.node(
            &plan.run_id,
            &artifact,
            "artifact",
            json!({"path":path,"sha256":sha,"status":"observed"}),
        )?;
        g.edge(
            &plan.run_id,
            &format!("run:{}:stage:{}", plan.run_id, stage.id),
            &artifact,
            "uses",
            json!({}),
        )?;
    }

    let previous: Option<(String, i64, Option<String>)> =
        g.db.query_row(
            "SELECT status,attempts,receipt FROM operator_stages WHERE run_id=?1 AND id=?2",
            params![plan.run_id, stage.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let previous_attempts = previous.as_ref().map_or(0, |x| x.1 as usize);
    if let Some((state, _, Some(receipt))) = &previous {
        let r: Value = serde_json::from_str(receipt)?;
        if state == "passed"
            && !stage.outputs.is_empty()
            && r["inputs"] == serde_json::to_value(&inputs)?
            && r["program_sha256"] == serde_json::to_value(&program_sha256)?
        {
            if let Ok(outputs) = artifacts(&root, &stage.outputs) {
                if r["outputs"] == serde_json::to_value(outputs)? {
                    g.record(
                        &plan.run_id,
                        "stage_reused",
                        json!({"stage":stage.id,"reason":"declared input/output hashes match"}),
                    )?;
                    return Ok(true);
                }
            }
        }
    }
    if previous_attempts >= stage.max_attempts {
        rejection(
            &mut g,
            &plan.run_id,
            "attempt budget exhausted",
            Some(&stage.id),
        )?;
        return Ok(false);
    }
    for attempt in previous_attempts + 1..=stage.max_attempts {
        let mut args = stage.args.clone();
        if attempt > 1 && !stage.resume_args.is_empty() {
            // Only the explicit --resume PATH contract is supported. The trainer
            // validates checkpoint computation/data identity before loading state.
            if stage.resume_args.len() != 2 || stage.resume_args[0] != "--resume" {
                bail!("resume_args must be --resume PATH");
            }
            let checkpoint = root.join(&stage.resume_args[1]);
            if checkpoint.is_file() {
                if checkpoint.metadata()?.len() == 0 {
                    bail!("empty recovery checkpoint; refusing unsafe recovery");
                }
                args.extend(stage.resume_args.clone());
            }
            g.record(&plan.run_id, "recovery_selected", json!({"stage":stage.id,"attempt":attempt,"checkpoint_present":checkpoint.is_file(),"action":if checkpoint.is_file(){"trainer_validates_then_resumes"}else{"restart_without_checkpoint"}}))?;
        }
        let check = Check {
            id: stage.id.clone(),
            kind: "test".into(),
            program: stage.program.clone(),
            args,
            project_marker: None,
        };
        let container_name = crate::isolation::Container::name(
            &format!("{}:{}", root.display(), plan.run_id),
            &stage.id,
        );
        let check = if let Some(container) = &stage.container {
            crate::isolation::Container::cleanup(&container_name)?;
            container.wrap(&root, &container_name, &check)?
        } else {
            check
        };
        let logs = log_root(db, &plan.run_id)?
            .join(&stage.id)
            .join(format!("attempt-{attempt}"));
        fs::create_dir_all(&logs)?;
        g.db.execute("INSERT INTO operator_stages(run_id,id,status,attempts) VALUES(?1,?2,'running',?3) ON CONFLICT(run_id,id) DO UPDATE SET status='running',attempts=?3",params![plan.run_id,stage.id,attempt])?;
        g.node(&plan.run_id,&format!("run:{}:stage:{}",plan.run_id,stage.id),"execution",json!({"state":"running","operation":stage.operation,"attempt":attempt,"status_evidence":"observed"}))?;
        g.record(
            &plan.run_id,
            "stage_started",
            json!({"stage":stage.id,"attempt":attempt,"logs":logs}),
        )?;
        let started = Instant::now();
        let result = code::run_observed(
            &root,
            &check,
            "operator",
            &stage.id,
            Duration::from_secs(stage.timeout_seconds),
            Some(&logs),
            Some(cancel.clone()),
        );
        if stage.container.is_some() {
            crate::isolation::Container::cleanup(&container_name)?;
        }
        let result = match result {
            Ok(v) => v,
            Err(e) => {
                rejection(&mut g, &plan.run_id, &e.to_string(), Some(&stage.id))?;
                g.db.execute(
                    "UPDATE operator_stages SET status='failed' WHERE run_id=?1 AND id=?2",
                    params![plan.run_id, stage.id],
                )?;
                return Ok(false);
            }
        };
        let outputs = if result.status == "passed" {
            artifacts(&root, &stage.outputs).ok()
        } else {
            None
        };
        let passed = result.status == "passed" && outputs.is_some();
        let receipt = json!({"schema_version":1,"run_id":plan.run_id,"stage":stage.id,"attempt":attempt,"program_sha256":program_sha256,"inputs":inputs,"outputs":outputs,"execution":result,"wall_seconds":started.elapsed().as_secs_f64(),"performance_scope":"wall time only; CPU/GPU throughput requires explicit instrumented metrics","logs":logs});
        fs::write(
            logs.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt)?,
        )?;
        g.db.execute(
            "UPDATE operator_stages SET status=?3,receipt=?4 WHERE run_id=?1 AND id=?2",
            params![
                plan.run_id,
                stage.id,
                if passed { "passed" } else { "failed" },
                receipt.to_string()
            ],
        )?;
        g.record(&plan.run_id, "stage_finished", receipt.clone())?;
        g.node(&plan.run_id,&format!("run:{}:stage:{}",plan.run_id,stage.id),"execution",json!({"state":if passed{"passed"}else{"failed"},"attempt":attempt,"receipt_sha256":hash_file(&logs.join("receipt.json"))?}))?;
        if let Some(outputs) = outputs {
            for (path, hash) in outputs {
                let artifact = format!("artifact:{hash}");
                g.node(
                    &plan.run_id,
                    &artifact,
                    "artifact",
                    json!({"path":path,"sha256":hash,"status":"observed"}),
                )?;
                g.edge(
                    &plan.run_id,
                    &artifact,
                    &format!("run:{}:stage:{}", plan.run_id, stage.id),
                    "produced_by",
                    json!({}),
                )?;
            }
        }
        if passed {
            return Ok(true);
        }
        let retry = result
            .exit_code
            .is_some_and(|c| stage.retry_exit_codes.contains(&c))
            && attempt < stage.max_attempts
            && !cancel.load(Ordering::SeqCst);
        if !retry {
            rejection(
                &mut g,
                &plan.run_id,
                if result.status == "passed" {
                    "expected output missing or invalid"
                } else {
                    &result.status
                },
                Some(&stage.id),
            )?;
            return Ok(false);
        }
        g.record(&plan.run_id,"retry_scheduled",json!({"stage":stage.id,"attempt":attempt,"reason":"explicit retryable exit code","backoff_seconds":attempt}))?;
        for _ in 0..attempt * 10 {
            if cancel.load(Ordering::SeqCst) {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(false)
}
#[cfg(unix)]
pub async fn run(db: &str, plan: Plan, trusted: bool) -> Result<Value> {
    use std::os::fd::AsRawFd;
    plan.validate()?;
    if !trusted {
        bail!("--trusted-local required for execution")
    }
    let root = plan.workspace.canonicalize()?;
    if !root.is_dir() {
        bail!("workspace must be a directory")
    }
    let lock_path = format!(
        "{}.{}.lock",
        db,
        crate::ingest::hash(plan.run_id.as_bytes())
    );
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another controller owns this run")
    }
    let mut g = open(db)?;
    let raw = serde_json::to_string(&plan)?;
    let hash = crate::ingest::hash(raw.as_bytes());
    let previous: Option<String> =
        g.db.query_row(
            "SELECT plan_hash FROM operator_runs WHERE id=?1",
            [&plan.run_id],
            |r| r.get(0),
        )
        .optional()?;
    if previous.as_ref().is_some_and(|h| h != &hash) {
        bail!("run ID reused with different plan")
    }
    g.db.execute(
        "INSERT OR IGNORE INTO operator_runs(id,plan_hash,plan,status) VALUES(?1,?2,?3,'created')",
        params![plan.run_id, hash, raw],
    )?;
    let state = status(db, &plan.run_id)?;
    if state["status"] == "completed" {
        for row in state["stages"].as_array().unwrap() {
            let id = row["id"].as_str().unwrap();
            let stage = plan.stages.iter().find(|s| s.id == id).unwrap();
            if row["status"] != "passed"
                || serde_json::to_value(artifacts(&root, &stage.inputs)?)?
                    != row["receipt"]["inputs"]
                || serde_json::to_value(executable_hash(
                    &root,
                    if stage.container.is_some() {
                        "docker"
                    } else {
                        &stage.program
                    },
                ))? != row["receipt"]["program_sha256"]
                || serde_json::to_value(artifacts(&root, &stage.outputs)?)?
                    != row["receipt"]["outputs"]
            {
                bail!(
                    "completed run input, executable or output identity changed; create a new run"
                )
            }
        }
        g.verify()?;
        return Ok(state);
    }
    if ["observe", "assist"].contains(&plan.mode.as_str()) {
        g.db.execute(
            "UPDATE operator_runs SET status='awaiting_execution_mode' WHERE id=?1",
            [&plan.run_id],
        )?;
        g.record(&plan.run_id,"proposal",json!({"mode":plan.mode,"execution":false,"plan_sha256":hash,"choices":["review_plan","create_bounded_plan"]}))?;
        return status(db, &plan.run_id);
    }
    for s in &plan.stages {
        if !plan.allowed_operations.contains(&s.operation) {
            g.db.execute(
                "UPDATE operator_runs SET status='rejected' WHERE id=?1",
                [&plan.run_id],
            )?;
            rejection(
                &mut g,
                &plan.run_id,
                "operation not allowed by plan policy",
                Some(&s.id),
            )?;
            return status(db, &plan.run_id);
        }
    }
    g.db.execute(
        "UPDATE operator_runs SET status='running' WHERE id=?1",
        [&plan.run_id],
    )?;
    g.node(
        &plan.run_id,
        &format!("run:{}", plan.run_id),
        "run",
        json!({"mode":plan.mode,"plan_sha256":hash,"state":"running"}),
    )?;
    for s in &plan.stages {
        for dep in &s.depends_on {
            g.edge(
                &plan.run_id,
                &format!("run:{}:stage:{}", plan.run_id, s.id),
                &format!("run:{}:stage:{}", plan.run_id, dep),
                "depends_on",
                json!({}),
            )?;
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let begin = Instant::now();
    let mut completed = BTreeSet::new();
    let mut launched = BTreeSet::new();
    let mut used = 0;
    let mut tasks = tokio::task::JoinSet::new();
    let mut failed = false;
    let mut final_control: String;
    loop {
        final_control = g.db.query_row(
            "SELECT control FROM operator_runs WHERE id=?1",
            [&plan.run_id],
            |r| r.get(0),
        )?;
        if begin.elapsed().as_secs() >= plan.max_wall_seconds {
            cancel.store(true, Ordering::SeqCst);
            failed = true;
            final_control = "budget_exhausted".into();
        }
        if final_control == "cancel" {
            cancel.store(true, Ordering::SeqCst)
        }
        if final_control == "run" && !failed {
            for s in &plan.stages {
                if !launched.contains(&s.id)
                    && s.depends_on.iter().all(|id| completed.contains(id))
                    && used + s.slots <= plan.max_slots
                {
                    launched.insert(s.id.clone());
                    used += s.slots;
                    let (db, p, s, flag) = (db.to_owned(), plan.clone(), s.clone(), cancel.clone());
                    tasks.spawn_blocking(move || {
                        let result = execute_stage(&db, &p, &s, flag);
                        (s.id, s.slots, result)
                    });
                }
            }
        }
        if tasks.is_empty() {
            break;
        }
        tokio::select! {
            joined=tasks.join_next()=>{let(id,slots,result)=joined.context("missing task")??;used-=slots;match result{Ok(true)=>{completed.insert(id);},Ok(false)=>failed=true,Err(e)=>{failed=true;rejection(&mut g,&plan.run_id,&e.to_string(),Some(&id))?;}}},
            _=tokio::time::sleep(Duration::from_millis(100))=>{}
        }
    }
    let state = if final_control == "cancel" {
        "cancelled"
    } else if final_control == "pause" {
        "paused"
    } else if failed || completed.len() != plan.stages.len() {
        "failed"
    } else {
        "completed"
    };
    g.db.execute(
        "UPDATE operator_runs SET status=?2 WHERE id=?1",
        params![plan.run_id, state],
    )?;
    g.node(
        &plan.run_id,
        &format!("run:{}", plan.run_id),
        "run",
        json!({"mode":plan.mode,"plan_sha256":hash,"state":state}),
    )?;
    g.record(&plan.run_id,"notification",json!({"severity":if state=="completed"{"info"}else{"warning"},"state":state,"seconds":begin.elapsed().as_secs_f64(),"completed_stages":completed.len()}))?;
    if let Err(e) = g.verify() {
        g.db.execute(
            "UPDATE operator_runs SET status='integrity_failed' WHERE id=?1",
            [&plan.run_id],
        )?;
        return Err(e);
    }
    status(db, &plan.run_id)
}
#[cfg(not(unix))]
pub async fn run(_db: &str, _plan: Plan, _trusted: bool) -> Result<Value> {
    bail!("operator currently requires Unix")
}

pub fn reference_plan(
    workspace: PathBuf,
    corpus: PathBuf,
    out: String,
    run_id: String,
    steps: usize,
    python: String,
) -> Result<Plan> {
    if steps < 2 || !relative(&out) {
        bail!("steps must be >=2 and output must be workspace-relative")
    }
    let native = std::env::current_exe()?.to_string_lossy().into_owned();
    let path = |name: &str| format!("{out}/{name}");
    let py = |id: &str,
              operation: &str,
              args: Vec<String>,
              deps: Vec<&str>,
              inputs: Vec<String>,
              outputs: Vec<String>| Stage {
        id: id.into(),
        operation: operation.into(),
        program: python.clone(),
        args,
        depends_on: deps.into_iter().map(String::from).collect(),
        inputs,
        outputs,
        slots: 1,
        max_attempts: 1,
        retry_exit_codes: vec![],
        resume_args: vec![],
        container: None,
        timeout_seconds: 300,
        metric_log: None,
    };
    let research = |module: &str, args: Vec<String>| {
        let mut all = vec!["scripts/research_entry.py".into(), module.into()];
        all.extend(args);
        all
    };
    let mut stages = vec![py(
        "prepare",
        "prepare",
        vec![
            "scripts/run_reference.py".into(),
            corpus.to_string_lossy().into(),
            "--out".into(),
            out.clone(),
            "--license-reference".into(),
            "Repository LICENSE; immutable source snapshot".into(),
            "--prepare-only".into(),
        ],
        vec![],
        vec![
            "scripts/run_reference.py".into(),
            "crates/aegis-data/src/ingest.rs".into(),
            "crates/aegis-data/src/tokenize.rs".into(),
        ],
        vec![
            path("sources.json"),
            path("prepared/manifest.json"),
            path("train.npy"),
            path("validation.npy"),
        ],
    )];
    if corpus.canonicalize()?
        == workspace
            .join("fixtures/reference-code")
            .canonicalize()
            .unwrap_or_default()
    {
        stages[0].args.extend([
            "--frozen-manifest".into(),
            "fixtures/reference-manifest.json".into(),
        ]);
        stages[0]
            .inputs
            .push("fixtures/reference-manifest.json".into());
    }
    stages.push(Stage {
        id: "validate".into(),
        operation: "validate".into(),
        program: native.clone(),
        args: vec!["validate-dataset".into(), path("prepared")],
        depends_on: vec!["prepare".into()],
        inputs: vec![path("prepared/manifest.json")],
        outputs: vec![],
        slots: 1,
        max_attempts: 5,
        retry_exit_codes: vec![],
        resume_args: vec![],
        container: None,
        timeout_seconds: 60,
        metric_log: None,
    });
    stages.push(py(
        "baseline",
        "evaluate",
        research(
            "aegis.heldout",
            vec![
                path("validation.npy"),
                "--out".into(),
                path("baseline.json"),
            ],
        ),
        vec!["validate"],
        vec![
            path("validation.npy"),
            path("validation.npy.manifest.json"),
            "research/aegis/heldout.py".into(),
            "research/aegis/model.py".into(),
            "research/aegis/identity.py".into(),
        ],
        vec![path("baseline.json")],
    ));
    stages.push(py(
        "train",
        "train",
        research(
            "aegis.train",
            vec![
                path("train.npy"),
                "--out".into(),
                path("continuous"),
                "--steps".into(),
                steps.to_string(),
                "--batch".into(),
                "2".into(),
                "--save-every".into(),
                "1".into(),
            ],
        ),
        vec!["baseline"],
        vec![
            path("train.npy"),
            path("train.npy.manifest.json"),
            "research/aegis/train.py".into(),
            "research/aegis/model.py".into(),
            "research/aegis/losses.py".into(),
            "research/aegis/identity.py".into(),
        ],
        vec![
            path("continuous/checkpoint.pt"),
            path("continuous/receipt.json"),
        ],
    ));
    stages.last_mut().unwrap().metric_log = Some(path("continuous/metrics.jsonl"));
    stages.last_mut().unwrap().max_attempts = 3;
    stages.last_mut().unwrap().resume_args =
        vec!["--resume".into(), path("continuous/checkpoint.pt")];
    stages.push(py(
        "prefix",
        "train",
        research(
            "aegis.train",
            vec![
                path("train.npy"),
                "--out".into(),
                path("prefix"),
                "--steps".into(),
                (steps / 2).to_string(),
                "--batch".into(),
                "2".into(),
            ],
        ),
        vec!["baseline"],
        vec![path("train.npy"), "research/aegis/train.py".into()],
        vec![path("prefix/checkpoint.pt")],
    ));
    stages.push(py(
        "resume",
        "resume",
        research(
            "aegis.train",
            vec![
                path("train.npy"),
                "--out".into(),
                path("resumed"),
                "--steps".into(),
                steps.to_string(),
                "--batch".into(),
                "2".into(),
                "--resume".into(),
                path("prefix/checkpoint.pt"),
            ],
        ),
        vec!["prefix"],
        vec![path("prefix/checkpoint.pt"), path("train.npy")],
        vec![path("resumed/checkpoint.pt")],
    ));
    stages.push(py(
        "recovery-proof",
        "verify",
        vec![
            "scripts/verify_resume.py".into(),
            path("continuous/checkpoint.pt"),
            path("resumed/checkpoint.pt"),
            "--out".into(),
            path("resume-verification.json"),
        ],
        vec!["train", "resume"],
        vec![
            path("continuous/checkpoint.pt"),
            path("resumed/checkpoint.pt"),
            "scripts/verify_resume.py".into(),
        ],
        vec![path("resume-verification.json")],
    ));
    stages.push(py(
        "candidate",
        "evaluate",
        research(
            "aegis.heldout",
            vec![
                path("validation.npy"),
                "--checkpoint".into(),
                path("continuous/checkpoint.pt"),
                "--out".into(),
                path("candidate.json"),
            ],
        ),
        vec!["train"],
        vec![
            path("validation.npy"),
            path("continuous/checkpoint.pt"),
            "research/aegis/heldout.py".into(),
        ],
        vec![path("candidate.json")],
    ));
    stages.push(py(
        "compare",
        "compare",
        vec![
            "scripts/compare_reference.py".into(),
            out.clone(),
            "--steps".into(),
            steps.to_string(),
        ],
        vec!["candidate", "recovery-proof"],
        vec![
            path("baseline.json"),
            path("candidate.json"),
            path("resume-verification.json"),
            "scripts/compare_reference.py".into(),
        ],
        vec![
            path("comparison.json"),
            path("gates.json"),
            path("experiment-baseline.json"),
            path("experiment-candidate.json"),
        ],
    ));
    stages.push(Stage {
        id: "accept".into(),
        operation: "accept".into(),
        program: native,
        args: vec![
            "gate".into(),
            path("gates.json"),
            "--out".into(),
            path("acceptance.json"),
        ],
        depends_on: vec!["compare".into()],
        inputs: vec![path("gates.json"), path("comparison.json")],
        outputs: vec![path("acceptance.json")],
        slots: 1,
        max_attempts: 1,
        retry_exit_codes: vec![],
        resume_args: vec![],
        container: None,
        timeout_seconds: 60,
        metric_log: None,
    });
    stages.push(py(
        "export",
        "export",
        vec![
            "scripts/export_run.py".into(),
            out.clone(),
            "--run-id".into(),
            run_id.clone(),
        ],
        vec!["accept"],
        vec![
            path("acceptance.json"),
            path("gates.json"),
            path("comparison.json"),
            path("continuous/checkpoint.pt"),
            "scripts/export_run.py".into(),
        ],
        vec![path("export/manifest.json"), path("export/checkpoint.pt")],
    ));
    let plan = Plan {
        run_id,
        workspace,
        mode: "bounded".into(),
        max_slots: 2,
        max_wall_seconds: 900,
        allowed_operations: vec![
            "prepare", "validate", "evaluate", "train", "resume", "verify", "compare", "accept",
            "export",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        stages,
    };
    plan.validate()?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(root: &Path) -> Plan {
        Plan {
            run_id: "test".into(),
            workspace: root.into(),
            mode: "bounded".into(),
            max_slots: 2,
            max_wall_seconds: 5,
            allowed_operations: vec!["test".into()],
            stages: vec![Stage {
                id: "one".into(),
                operation: "test".into(),
                program: "true".into(),
                args: vec![],
                depends_on: vec![],
                inputs: vec![],
                outputs: vec![],
                slots: 1,
                max_attempts: 1,
                retry_exit_codes: vec![],
                resume_args: vec![],
                container: None,
                timeout_seconds: 2,
                metric_log: None,
            }],
        }
    }
    #[test]
    fn dot_identifiers_cannot_escape_artifact_roots() {
        assert!(!identifier("."));
        assert!(!identifier(".."));
        assert!(identifier("run-01"));
    }
    #[test]
    fn cycle_rejected() {
        let d = tempfile::tempdir().unwrap();
        let mut p = plan(d.path());
        p.stages[0].depends_on = vec!["one".into()];
        assert!(p.validate().is_err());
    }
    #[tokio::test]
    async fn policy_and_execution() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("operator.db").to_str().unwrap().to_owned();
        let p = plan(d.path());
        assert_eq!(run(&db, p, true).await.unwrap()["status"], "completed");
        let mut p = plan(d.path());
        p.run_id = "rejected".into();
        p.allowed_operations.clear();
        assert_eq!(run(&db, p, true).await.unwrap()["status"], "rejected");
    }
}
