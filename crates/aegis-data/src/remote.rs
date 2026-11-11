//! Worker-scoped HTTP lease transport. Loopback server; TLS/SSH gateway for remote hosts.
use crate::{
    ingest::hash_file,
    ledger::{Lease, Ledger},
    operator::Plan,
    worker::{self, Payload},
};
use anyhow::{bail, Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const CHUNK: usize = 1024 * 1024;
const BUDGET: u64 = 256 * 1024 * 1024;
fn now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}
fn relative(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 512
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !p.contains('\\')
}
#[derive(Clone)]
struct App {
    db: String,
    tokens: Arc<BTreeMap<String, String>>,
    lock: Arc<Mutex<()>>,
    ttl: i64,
}
type Reply = std::result::Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (StatusCode::CONFLICT, Json(json!({"error":e.to_string()})))
}
fn auth(app: &App, h: &HeaderMap) -> Result<String> {
    let id = h
        .get("x-aegis-worker")
        .and_then(|x| x.to_str().ok())
        .unwrap_or("");
    let received = h
        .get("authorization")
        .and_then(|x| x.to_str().ok())
        .and_then(|x| x.strip_prefix("Bearer "))
        .unwrap_or("");
    let expected = app
        .tokens
        .get(id)
        .context("worker authentication required")?;
    let mut diff = received.len() ^ expected.len();
    for (i, b) in expected.bytes().enumerate() {
        diff |= usize::from(b ^ received.as_bytes().get(i).copied().unwrap_or(0));
    }
    if diff != 0 {
        bail!("worker authentication required")
    };
    Ok(id.into())
}
fn authenticate(
    app: &App,
    h: &HeaderMap,
) -> std::result::Result<String, (StatusCode, Json<Value>)> {
    auth(app, h).map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"worker authentication required"})),
        )
    })
}
fn live(app: &App, id: &str, lease: &Lease) -> Result<Payload> {
    if lease.worker != id {
        bail!("worker identity mismatch")
    };
    let row = Ledger::open(&app.db)?.inspect(&lease.id)?;
    if row["state"] != "leased"
        || row["worker"] != id
        || row["epoch"] != lease.epoch
        || row["expires"].as_i64().unwrap_or(0) <= now()?
    {
        bail!("stale or cancelled lease")
    }
    Ok(serde_json::from_str(
        row["payload"].as_str().context("payload")?,
    )?)
}
fn upload_root(app: &App, lease: &Lease) -> Result<PathBuf> {
    let root = PathBuf::from(format!("{}.remote-returns", app.db));
    fs::create_dir_all(&root)?;
    let root = root
        .canonicalize()?
        .join(&lease.id)
        .join(format!("epoch-{}", lease.epoch));
    fs::create_dir_all(&root)?;
    Ok(root)
}
fn output_names(plan: &Plan) -> BTreeSet<String> {
    plan.stages
        .iter()
        .flat_map(|s| s.outputs.iter().cloned())
        .collect()
}
#[derive(Serialize, Deserialize)]
struct Assignment {
    lease: Lease,
    plan: Plan,
    files: BTreeMap<String, String>,
    sizes: BTreeMap<String, u64>,
    ttl: i64,
}
async fn claim(State(app): State<App>, h: HeaderMap) -> Reply {
    let id = authenticate(&app, &h)?;
    let _guard = app.lock.lock().map_err(error)?;
    let mut ledger = Ledger::open(&app.db).map_err(error)?;
    let Some(lease) = ledger
        .claim(&id, now().map_err(error)?, app.ttl)
        .map_err(error)?
    else {
        return Ok(Json(json!({"state":"idle"})));
    };
    let payload = live(&app, &id, &lease).map_err(error)?;
    if worker::inventory(&payload.bundle).map_err(error)? != payload.files {
        return Err(error("bundle changed"));
    }
    let sizes = payload
        .files
        .keys()
        .map(|name| Ok((name.clone(), fs::metadata(payload.bundle.join(name))?.len())))
        .collect::<Result<_>>()
        .map_err(error)?;
    Ok(Json(
        serde_json::to_value(Assignment {
            lease,
            plan: payload.plan,
            files: payload.files,
            sizes,
            ttl: app.ttl,
        })
        .map_err(error)?,
    ))
}
async fn renew(State(app): State<App>, h: HeaderMap, Json(lease): Json<Lease>) -> Reply {
    let id = authenticate(&app, &h)?;
    live(&app, &id, &lease).map_err(error)?;
    let expires = Ledger::open(&app.db)
        .map_err(error)?
        .renew(&lease, now().map_err(error)?, app.ttl)
        .map_err(error)?;
    Ok(Json(json!({"expires":expires})))
}
#[derive(Serialize, Deserialize)]
struct Transfer {
    id: String,
    epoch: i64,
    path: String,
    offset: u64,
}
fn query_lease(id: &str, q: &Transfer) -> Lease {
    Lease {
        id: q.id.clone(),
        worker: id.into(),
        epoch: q.epoch,
        expires: 0,
    }
}
async fn input(
    State(app): State<App>,
    h: HeaderMap,
    Query(q): Query<Transfer>,
) -> std::result::Result<Vec<u8>, (StatusCode, Json<Value>)> {
    let id = authenticate(&app, &h)?;
    let payload = live(&app, &id, &query_lease(&id, &q)).map_err(error)?;
    if !relative(&q.path) || !payload.files.contains_key(&q.path) {
        return Err(error("unknown input"));
    }
    let root = payload.bundle.canonicalize().map_err(error)?;
    let path = root.join(&q.path).canonicalize().map_err(error)?;
    if !path.starts_with(root) {
        return Err(error("input escaped bundle"));
    }
    let mut file = fs::File::open(path).map_err(error)?;
    if q.offset > file.metadata().map_err(error)?.len() {
        return Err(error("invalid input offset"));
    }
    file.seek(SeekFrom::Start(q.offset)).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(CHUNK as u64)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    Ok(bytes)
}
fn used(root: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            bail!("unexpected symlink")
        };
        total += if meta.is_dir() {
            used(&path)?
        } else {
            meta.len()
        };
        if total > BUDGET {
            bail!("return quota exceeded")
        }
    }
    Ok(total)
}
async fn upload(
    State(app): State<App>,
    h: HeaderMap,
    Query(q): Query<Transfer>,
    body: Bytes,
) -> Reply {
    let id = authenticate(&app, &h)?;
    let _guard = app.lock.lock().map_err(error)?;
    let lease = query_lease(&id, &q);
    let payload = live(&app, &id, &lease).map_err(error)?;
    if !relative(&q.path) || !output_names(&payload.plan).contains(&q.path) || body.len() > CHUNK {
        return Err(error("unknown output or oversized chunk"));
    }
    let root = upload_root(&app, &lease).map_err(error)?.join("files");
    fs::create_dir_all(&root).map_err(error)?;
    let path = root.join(&q.path);
    fs::create_dir_all(path.parent().unwrap()).map_err(error)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(error)?;
    let len = file.metadata().map_err(error)?.len();
    if q.offset < len {
        file.seek(SeekFrom::Start(q.offset)).map_err(error)?;
        let mut old = vec![0; body.len()];
        file.read_exact(&mut old).map_err(error)?;
        if old != body {
            return Err(error("conflicting retransmission"));
        };
        return Ok(Json(
            json!({"offset":q.offset+body.len() as u64,"reused":true}),
        ));
    }
    if q.offset != len || used(&root).map_err(error)? + body.len() as u64 > BUDGET {
        return Err(error("invalid offset or return quota exceeded"));
    }
    file.seek(SeekFrom::End(0)).map_err(error)?;
    file.write_all(&body).map_err(error)?;
    file.sync_data().map_err(error)?;
    Ok(Json(json!({"offset":len+body.len() as u64})))
}
#[derive(Serialize, Deserialize)]
struct Completion {
    lease: Lease,
    outputs: BTreeMap<String, String>,
}
async fn complete(State(app): State<App>, h: HeaderMap, Json(c): Json<Completion>) -> Reply {
    let id = authenticate(&app, &h)?;
    let _guard = app.lock.lock().map_err(error)?;
    let ledger = Ledger::open(&app.db).map_err(error)?;
    let state = ledger.inspect(&c.lease.id).map_err(error)?;
    if state["state"] == "done" && state["worker"] == id && state["epoch"] == c.lease.epoch {
        let result: Value =
            serde_json::from_str(state["result"].as_str().unwrap()).map_err(error)?;
        if result["outputs"] == serde_json::to_value(&c.outputs).map_err(error)? {
            return Ok(Json(result));
        }
        return Err(error("conflicting completed return"));
    }
    let payload = live(&app, &id, &c.lease).map_err(error)?;
    if c.outputs.keys().cloned().collect::<BTreeSet<_>>() != output_names(&payload.plan) {
        return Err(error("return membership mismatch"));
    }
    let root = upload_root(&app, &c.lease).map_err(error)?;
    let files = root.join("files");
    if !files.exists() {
        fs::create_dir_all(&files).map_err(error)?;
    }
    if worker::inventory(&files).map_err(error)? != c.outputs {
        return Err(error("returned hashes mismatch"));
    }
    let result = json!({"schema_version":1,"id":c.lease.id,"worker":id,"epoch":c.lease.epoch,"outputs":c.outputs,"root":files,"transport_verified":true,"scope":"trusted worker assertion plus controller-verified bytes; not independent model quality attestation"});
    let receipt = root.join("receipt.json");
    let tmp = root.join("receipt.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&result).map_err(error)?).map_err(error)?;
    fs::File::open(&tmp)
        .map_err(error)?
        .sync_all()
        .map_err(error)?;
    fs::rename(tmp, &receipt).map_err(error)?;
    ledger
        .complete(
            &c.lease,
            now().map_err(error)?,
            &serde_json::to_string(&result).map_err(error)?,
        )
        .map_err(error)?;
    Ok(Json(result))
}
async fn fail(State(app): State<App>, h: HeaderMap, Json(lease): Json<Lease>) -> Reply {
    let id = authenticate(&app, &h)?;
    live(&app, &id, &lease).map_err(error)?;
    Ledger::open(&app.db)
        .map_err(error)?
        .fail(
            &lease,
            now().map_err(error)?,
            "remote worker reported execution failure",
        )
        .map_err(error)?;
    Ok(Json(json!({"failed":true})))
}
pub async fn serve(db: String, tokens: PathBuf, bind: SocketAddr, ttl: i64) -> Result<()> {
    if !bind.ip().is_loopback() || !(6..=300).contains(&ttl) {
        bail!("loopback bind and TTL 6..300 required; use TLS gateway/SSH tunnel for remote access")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&tokens)?.permissions().mode() & 0o077 != 0 {
            bail!("worker token file must be owner-only")
        }
    }
    let tokens: BTreeMap<String, String> = serde_json::from_slice(&fs::read(tokens)?)?;
    if tokens.is_empty()
        || tokens.iter().any(|(id, t)| {
            id.is_empty()
                || id.len() > 80
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                || t.len() < 32
        })
    {
        bail!("invalid worker registry")
    }
    let unique: BTreeSet<_> = tokens.values().collect();
    if unique.len() != tokens.len() {
        bail!("worker tokens must be distinct")
    }
    Ledger::open(&db)?;
    let app = App {
        db,
        tokens: Arc::new(tokens),
        lock: Arc::new(Mutex::new(())),
        ttl,
    };
    let router = Router::new()
        .route(
            "/health",
            get(|| async {
                Json(json!({"status":"ok","service":"aegis-worker-controller","version":1}))
            }),
        )
        .route("/v1/claim", post(claim))
        .route("/v1/renew", post(renew))
        .route("/v1/input", get(input))
        .route("/v1/upload", post(upload))
        .route("/v1/complete", post(complete))
        .route("/v1/fail", post(fail))
        .layer(DefaultBodyLimit::max(CHUNK))
        .with_state(app);
    axum::serve(tokio::net::TcpListener::bind(bind).await?, router).await?;
    Ok(())
}

struct Client {
    http: reqwest::blocking::Client,
    base: String,
    worker: String,
    token: String,
}
impl Client {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("x-aegis-worker", &self.worker)
    }
    fn json<T: serde::de::DeserializeOwned>(
        &self,
        r: reqwest::blocking::RequestBuilder,
    ) -> Result<T> {
        let mut response = r.send()?.error_for_status()?;
        let mut bytes = Vec::new();
        (&mut response)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            bail!("controller response limit")
        };
        Ok(serde_json::from_slice(&bytes)?)
    }
    fn beat(&self, a: &Assignment, last: &mut Instant) -> Result<()> {
        if last.elapsed() >= Duration::from_millis(a.ttl as u64 * 250) {
            let _: Value = self.json(
                self.request(reqwest::Method::POST, "/v1/renew")
                    .json(&a.lease),
            )?;
            *last = Instant::now();
        }
        Ok(())
    }
}
pub fn run(
    controller: &str,
    worker_id: &str,
    root: &Path,
    token_env: &str,
    trusted: bool,
) -> Result<Value> {
    let url = reqwest::Url::parse(controller)?;
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost" || h.parse::<std::net::IpAddr>().is_ok_and(|i| i.is_loopback())
    });
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
    {
        bail!("HTTPS controller or HTTP loopback tunnel required; no URL credentials/query/path")
    }
    let token = std::env::var(token_env).context("worker token environment variable missing")?;
    if token.len() < 32 {
        bail!("worker token too short")
    }
    let client = Client {
        http: reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(3))
            .no_proxy()
            .build()?,
        base: controller.trim_end_matches('/').into(),
        worker: worker_id.into(),
        token,
    };
    let value: Value = client.json(client.request(reqwest::Method::POST, "/v1/claim"))?;
    if value["state"] == "idle" {
        return Ok(value);
    }
    let a: Assignment = serde_json::from_value(value)?;
    let result = (|| -> Result<Value> {
        a.plan.validate()?;
        if a.lease.worker != worker_id
            || a.lease.id != a.plan.run_id
            || !(1..=3).contains(&a.lease.epoch)
            || a.files.keys().any(|p| !relative(p))
            || a.files.keys().ne(a.sizes.keys())
            || a.sizes
                .values()
                .try_fold(0u64, |s, n| s.checked_add(*n))
                .context("input size overflow")?
                > BUDGET
        {
            bail!("invalid remote assignment")
        }
        if !trusted && a.plan.stages.iter().any(|s| s.container.is_none()) {
            bail!("remote stages require container isolation unless --trusted-local is explicit")
        }
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        // Epoch directory is create-new. A restarted remote client never reuses mutable files.
        let epoch = root.join(format!("{}-e{}", a.lease.id, a.lease.epoch));
        fs::create_dir(&epoch)?;
        let bundle = epoch.join("bundle");
        fs::create_dir(&bundle)?;
        let mut last = Instant::now();
        for (name, sha) in &a.files {
            let path = bundle.join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            let mut offset = 0;
            while offset < a.sizes[name] {
                client.beat(&a, &mut last)?;
                let q = Transfer {
                    id: a.lease.id.clone(),
                    epoch: a.lease.epoch,
                    path: name.clone(),
                    offset,
                };
                let response = client
                    .request(reqwest::Method::GET, "/v1/input")
                    .query(&q)
                    .send()?
                    .error_for_status()?;
                let mut chunk = Vec::new();
                response.take(CHUNK as u64 + 1).read_to_end(&mut chunk)?;
                if chunk.is_empty()
                    || chunk.len() > CHUNK
                    || offset + chunk.len() as u64 > a.sizes[name]
                {
                    bail!("invalid input transfer")
                };
                file.write_all(&chunk)?;
                offset += chunk.len() as u64;
            }
            file.sync_all()?;
            if hash_file(&path)? != *sha {
                bail!("downloaded input hash mismatch")
            }
        }
        let mut plan = a.plan.clone();
        plan.workspace = bundle.clone();
        plan.run_id = format!("remote-e{}", a.lease.epoch);
        let plan_path = epoch.join("plan.json");
        fs::write(&plan_path, serde_json::to_vec_pretty(&plan)?)?;
        let db = epoch.join("worker.db");
        let db = db.to_str().context("UTF8 worker path required")?;
        worker::enqueue(db, &plan_path, &bundle)?;
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command
            .args([
                "worker-once",
                db,
                "remote-local",
                "--ttl",
                "6",
                "--trusted-local",
            ])
            .env_clear();
        for key in ["PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME"] {
            if let Some(v) = std::env::var_os(key) {
                command.env(key, v);
            }
        }
        command
            .stdout(std::process::Stdio::from(fs::File::create(
                epoch.join("worker.stdout.log"),
            )?))
            .stderr(std::process::Stdio::from(fs::File::create(
                epoch.join("worker.stderr.log"),
            )?));
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
                            return Err(std::io::Error::other("remote supervisor lost"));
                        }
                        Ok(())
                    });
                }
            }
        }
        let mut child = command.spawn()?;
        let started = Instant::now();
        loop {
            if let Err(e) = client.beat(&a, &mut last) {
                let _ = Ledger::open(db)?.cancel(&plan.run_id);
                let deadline = Instant::now();
                while child.try_wait()?.is_none() && deadline.elapsed() < Duration::from_secs(3) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                if child.try_wait()?.is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                return Err(e);
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("remote execution failed; inspect worker epoch logs")
                };
                break;
            }
            if started.elapsed() > Duration::from_secs(plan.max_wall_seconds + 15) {
                let _ = Ledger::open(db)?.cancel(&plan.run_id);
                let _ = child.kill();
                let _ = child.wait();
                bail!("remote execution deadline")
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        worker::verify(db, &plan.run_id)?;
        let state = Ledger::open(db)?.inspect(&plan.run_id)?;
        let result: Value = serde_json::from_str(state["result"].as_str().unwrap())?;
        let receipt: Value =
            serde_json::from_slice(&fs::read(result["receipt"].as_str().unwrap())?)?;
        let workspace = Path::new(receipt["workspace"].as_str().unwrap());
        let outputs: BTreeMap<String, String> = serde_json::from_value(receipt["outputs"].clone())?;
        for name in outputs.keys() {
            let mut file = fs::File::open(workspace.join(name))?;
            let mut offset = 0;
            loop {
                client.beat(&a, &mut last)?;
                let mut bytes = vec![0; CHUNK];
                let n = file.read(&mut bytes)?;
                bytes.truncate(n);
                if n == 0 && offset > 0 {
                    break;
                }
                let q = Transfer {
                    id: a.lease.id.clone(),
                    epoch: a.lease.epoch,
                    path: name.clone(),
                    offset,
                };
                let _: Value = client.json(
                    client
                        .request(reqwest::Method::POST, "/v1/upload")
                        .query(&q)
                        .body(bytes),
                )?;
                offset += n as u64;
                if n == 0 {
                    break;
                }
            }
        }
        client.beat(&a, &mut last)?;
        let result: Value =
            client.json(client.request(reqwest::Method::POST, "/v1/complete").json(
                &Completion {
                    lease: a.lease.clone(),
                    outputs,
                },
            ))?;
        fs::write(
            epoch.join("controller-return.json"),
            serde_json::to_vec_pretty(&result)?,
        )?;
        Ok(result)
    })();
    if result.is_err() {
        let _ = client.json::<Value>(
            client
                .request(reqwest::Method::POST, "/v1/fail")
                .json(&a.lease),
        );
    }
    result
}
pub fn verify_return(db: &str, id: &str) -> Result<Value> {
    let state = Ledger::open(db)?.inspect(id)?;
    if state["state"] != "done" {
        bail!("remote result not committed")
    }
    let result: Value = serde_json::from_str(state["result"].as_str().context("remote result")?)?;
    if result["epoch"] != state["epoch"]
        || result["worker"] != state["worker"]
        || result["id"] != id
    {
        bail!("remote fence mismatch")
    }
    let files: BTreeMap<String, String> = serde_json::from_value(result["outputs"].clone())?;
    if worker::inventory(Path::new(result["root"].as_str().context("remote root")?))? != files {
        bail!("returned bytes changed")
    }
    Ok(
        json!({"verified":true,"id":id,"epoch":state["epoch"],"outputs":files,"scope":"controller verified transport integrity; execution and quality require trusted worker and operator evidence"}),
    )
}
