//! Authenticated loopback API for human and AI operators. No external delivery.
use crate::{
    graph::GraphStore,
    operator::{self, Plan},
};
use anyhow::{bail, Result};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};
#[derive(Clone)]
struct App {
    editor_assets: Option<Arc<crate::editor_assets::Assets>>,
    model_adapters: Arc<Vec<crate::model::Adapter>>,
    model_slots: Arc<tokio::sync::Semaphore>,
    language_servers: Arc<Vec<crate::language::Server>>,
    language_slots: Arc<tokio::sync::Semaphore>,
    capacity: Option<crate::capacity::Pool>,
    profile: Option<Arc<crate::runtime::Profile>>,
    db: String,
    workspace: PathBuf,
    token: Arc<String>,
    runs: Arc<tokio::sync::Semaphore>,
}
type Response = std::result::Result<Json<Value>, (StatusCode, Json<Value>)>;
fn error(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":e.to_string()})),
    )
}
fn auth(app: &App, headers: &HeaderMap) -> std::result::Result<(), (StatusCode, Json<Value>)> {
    let received = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let expected = app.token.as_bytes();
    let bytes = received.as_bytes();
    let mut difference = bytes.len() ^ expected.len();
    for (i, b) in expected.iter().enumerate() {
        difference |= usize::from(*b ^ bytes.get(i).copied().unwrap_or(0));
    }
    if difference == 0 {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"authentication required"})),
        ))
    }
}
async fn capabilities(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(crate::capabilities::catalog()))
}
async fn knowledge_build(
    State(app): State<App>,
    headers: HeaderMap,
    Json(documents): Json<Vec<crate::knowledge::Document>>,
) -> Response {
    auth(&app, &headers)?;
    let result = tokio::task::spawn_blocking(move || crate::knowledge::build(documents))
        .await
        .map_err(error)?
        .map_err(error)?;
    Ok(Json(serde_json::to_value(result).map_err(error)?))
}
async fn knowledge_context(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<crate::knowledge::ContextRequest>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        tokio::task::spawn_blocking(move || crate::knowledge::context(request))
            .await
            .map_err(error)?
            .map_err(error)?,
    ))
}
async fn chat_export(
    State(app): State<App>,
    headers: HeaderMap,
    Json(chats): Json<Vec<crate::knowledge::ReviewedChat>>,
) -> Response {
    auth(&app, &headers)?;
    let bytes = tokio::task::spawn_blocking(move || crate::knowledge::export(chats))
        .await
        .map_err(error)?
        .map_err(error)?;
    Ok(Json(
        json!({"schema_version":1,"sha256":crate::ingest::hash(&bytes),"jsonl":String::from_utf8(bytes).map_err(error)?,"training_started":false}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageRequest {
    server_id: String,
    query: crate::language::Query,
}
async fn language_query(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<LanguageRequest>,
) -> Response {
    auth(&app, &headers)?;
    let server = app
        .language_servers
        .iter()
        .find(|s| s.id == request.server_id)
        .cloned()
        .ok_or_else(|| error("language server not configured"))?;
    let permit = app
        .language_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"language analysis capacity exhausted"})),
            )
        })?;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::language::query(&app.workspace, &server, request.query)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelRequest {
    adapter_id: String,
    request: crate::model::Request,
}
struct DisconnectCancel(Arc<std::sync::atomic::AtomicBool>);
impl Drop for DisconnectCancel {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
async fn model_catalog(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        json!({"schema_version":1,"adapters":app.model_adapters.iter().map(|a|json!({"adapter_id":a.id,"model_id":a.model_id,"model_revision":a.model_revision,"transport":"aegis-jsonrpc-stdio-v1"})).collect::<Vec<_>>()}),
    ))
}
async fn model_stream(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<ModelRequest>,
) -> std::result::Result<
    axum::response::Sse<
        impl futures_util::Stream<
            Item = std::result::Result<axum::response::sse::Event, std::convert::Infallible>,
        >,
    >,
    (StatusCode, Json<Value>),
> {
    auth(&app, &headers)?;
    let adapter = app
        .model_adapters
        .iter()
        .find(|a| a.id == request.adapter_id)
        .cloned()
        .ok_or_else(|| error("model adapter not configured"))?;
    let permit = app.model_slots.clone().try_acquire_owned().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"model adapter capacity exhausted"})),
        )
    })?;
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = DisconnectCancel(cancel.clone());
    let (sender, receiver) = tokio::sync::mpsc::channel::<Value>(16);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let delivery_deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(adapter.timeout_seconds);
        let result = crate::model::generate(
            &app.workspace,
            &adapter,
            request.request,
            cancel.clone(),
            |mut event| loop {
                if cancel.load(std::sync::atomic::Ordering::Relaxed)
                    || std::time::Instant::now() >= delivery_deadline
                {
                    anyhow::bail!("stream cancelled or delivery deadline exceeded")
                }
                match sender.try_send(event) {
                    Ok(()) => return Ok(()),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        anyhow::bail!("stream consumer disconnected")
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Full(value)) => {
                        event = value;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            },
        );
        if let Err(error) = result {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = sender.try_send(json!({"type":"failed","error":error.to_string()}));
        }
    });
    let stream =
        futures_util::stream::unfold((receiver, guard), |(mut receiver, guard)| async move {
            receiver.recv().await.map(|event| {
                (
                    Ok(axum::response::sse::Event::default().data(event.to_string())),
                    (receiver, guard),
                )
            })
        });
    Ok(axum::response::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
async fn editor_asset(
    State(app): State<App>,
    Path(path): Path<String>,
) -> std::result::Result<axum::response::Response, StatusCode> {
    let bundle = app.editor_assets.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let asset = bundle.files.get(&path).ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        [
            ("content-type", asset.mime),
            ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
        ],
        asset.bytes.clone(),
    )
        .into_response())
}
async fn editor_asset_status(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        app.editor_assets
            .as_ref()
            .map_or_else(|| json!({"enabled":false}), |b| b.summary()),
    ))
}
async fn wire_validate(
    State(app): State<App>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    auth(&app, &headers)?;
    let packet = crate::packet::unpack(&body).map_err(error)?;
    Ok(Json(crate::packet::summary(&packet)))
}
async fn run_status(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(operator::status(&app.db, &id).map_err(error)?))
}
#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: i64,
}
async fn events(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<Cursor>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        operator::open(&app.db)
            .map_err(error)?
            .events(&id, q.after)
            .map_err(error)?
            .into(),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoverRequest {
    workspace: PathBuf,
    run_id: String,
}
async fn discover_project(
    State(app): State<App>,
    headers: HeaderMap,
    Json(request): Json<DiscoverRequest>,
) -> Response {
    auth(&app, &headers)?;
    let root = request.workspace.canonicalize().map_err(error)?;
    if !root.starts_with(&app.workspace) {
        return Err(error("workspace outside server root"));
    }
    let report =
        tokio::task::spawn_blocking(move || crate::discovery::discover(&root, &request.run_id))
            .await
            .map_err(error)?
            .map_err(error)?;
    Ok(Json(report))
}
#[derive(Deserialize)]
struct EditorQuery {
    path: Option<String>,
    q: Option<String>,
    mode: Option<String>,
}
async fn editor_tree(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        tokio::task::spawn_blocking(move || crate::editor::tree(&app.workspace))
            .await
            .map_err(error)?
            .map_err(error)?,
    ))
}
async fn editor_read(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<EditorQuery>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        crate::editor::read(&app.workspace, q.path.as_deref().unwrap_or("")).map_err(error)?,
    ))
}
async fn editor_search(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<EditorQuery>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            crate::editor::search(&app.workspace, q.q.as_deref().unwrap_or(""))
        })
        .await
        .map_err(error)?
        .map_err(error)?,
    ))
}
async fn editor_git(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<EditorQuery>,
) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        tokio::task::spawn_blocking(move || {
            crate::editor::git(&app.workspace, q.mode.as_deref().unwrap_or("status"))
        })
        .await
        .map_err(error)?
        .map_err(error)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditorWrite {
    path: String,
    expected_sha256: String,
    text: Option<String>,
    snapshot: Option<String>,
}
async fn editor_save(
    State(app): State<App>,
    headers: HeaderMap,
    Json(write): Json<EditorWrite>,
) -> Response {
    auth(&app, &headers)?;
    if write.text.is_some() == write.snapshot.is_some() {
        return Err(error("supply exactly one of text or snapshot"));
    }
    let result = tokio::task::spawn_blocking(move || match write.snapshot {
        Some(snapshot) => crate::editor::rollback(
            &app.workspace,
            &write.path,
            &write.expected_sha256,
            &snapshot,
        ),
        None => crate::editor::save(
            &app.workspace,
            &write.path,
            &write.expected_sha256,
            write.text.as_deref().unwrap(),
        ),
    })
    .await
    .map_err(error)?;
    result.map(Json).map_err(|e| {
        let status = if e.to_string().contains("conflict")
            || e.to_string().contains("owns this workspace")
        {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({"error":e.to_string()})))
    })
}
async fn use_cases(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(crate::uses::catalog()))
}
async fn runtime_profile(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        json!({"profile":app.profile.as_deref(),"scope":"per-run admission","active_run_limit":2,"capacity":app.capacity.as_ref().map(|pool| pool.snapshot()).transpose().map_err(error)?}),
    ))
}
async fn readiness(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        crate::readiness::report(&app.workspace, &app.workspace.join("readiness.json"))
            .map_err(error)?,
    ))
}
async fn graph(State(app): State<App>, headers: HeaderMap) -> Response {
    auth(&app, &headers)?;
    Ok(Json(
        GraphStore::open(&app.db)
            .map_err(error)?
            .snapshot(None)
            .map_err(error)?,
    ))
}
#[derive(Deserialize)]
struct Action {
    action: String,
    actor: String,
}
async fn control(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(a): Json<Action>,
) -> Response {
    auth(&app, &headers)?;
    operator::control(&app.db, &id, &a.action, &a.actor).map_err(error)?;
    Ok(Json(
        json!({"accepted":true,"pause_semantics":"stage_boundary","resume_execution":"POST saved plan to /v1/runs after control=run"}),
    ))
}
#[derive(Deserialize)]
struct Note {
    actor: String,
    text: String,
}
async fn note(
    State(app): State<App>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(n): Json<Note>,
) -> Response {
    auth(&app, &headers)?;
    if n.text.len() > 16384 {
        return Err(error("note too large"));
    };
    let seq = operator::open(&app.db)
        .map_err(error)?
        .record(
            &id,
            "operator_note",
            json!({"actor":n.actor,"text":n.text,"effect":"recorded context only"}),
        )
        .map_err(error)?;
    Ok(Json(json!({"seq":seq})))
}
async fn start(State(app): State<App>, headers: HeaderMap, Json(mut plan): Json<Plan>) -> Response {
    auth(&app, &headers)?;
    plan.validate().map_err(error)?;
    plan.workspace = plan.workspace.canonicalize().map_err(error)?;
    if !plan.workspace.starts_with(&app.workspace) {
        return Err(error("workspace outside server root"));
    };
    let admission = app
        .profile
        .as_ref()
        .map(|profile| profile.admit(&plan))
        .transpose()
        .map_err(error)?;
    let permit = app.runs.clone().try_acquire_owned().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"active run limit reached"})),
        )
    })?;
    let reservation = if let Some(pool) = &app.capacity {
        let receipt = admission
            .as_ref()
            .ok_or_else(|| error("capacity requires runtime profile"))?;
        Some(
            pool.reserve(
                receipt["container_cpu_upper_bound"].as_u64().unwrap_or(0),
                receipt["container_memory_mib_upper_bound"]
                    .as_u64()
                    .unwrap_or(0),
            )
            .map_err(|e| {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error":e.to_string()})),
                )
            })?,
        )
    } else {
        None
    };
    let run = plan.run_id.clone();
    let response_run = run.clone();
    let db = app.db.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let _reservation = reservation;
        if let Err(e) = operator::run(&db, plan, true).await {
            if let Ok(mut g) = operator::open(&db) {
                let _ = g.record(
                    &run,
                    "notification",
                    json!({"severity":"error","message":e.to_string()}),
                );
            }
        }
    });
    Ok(Json(
        json!({"status":"submitted","run_id":response_run,"admission":admission}),
    ))
}
#[derive(Deserialize)]
struct LogQuery {
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_limit")]
    limit: u64,
    #[serde(default = "stdout")]
    stream: String,
}
fn default_limit() -> u64 {
    16384
}
fn stdout() -> String {
    "stdout".into()
}
async fn logs(
    State(app): State<App>,
    headers: HeaderMap,
    Path((run, stage, attempt)): Path<(String, String, usize)>,
    Query(q): Query<LogQuery>,
) -> Response {
    auth(&app, &headers)?;
    if !["stdout", "stderr"].contains(&q.stream.as_str())
        || !stage
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        || stage == ".."
        || stage == "."
        || stage.is_empty()
    {
        return Err(error("invalid log stream/stage"));
    };
    let root = operator::log_root(&app.db, &run).map_err(error)?;
    let path = root
        .join(stage)
        .join(format!("attempt-{attempt}"))
        .join(format!("{}.log", q.stream));
    let mut file = std::fs::File::open(path).map_err(error)?;
    file.seek(SeekFrom::Start(q.offset)).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(q.limit.min(65536))
        .read_to_end(&mut bytes)
        .map_err(error)?;
    Ok(Json(
        json!({"offset":q.offset,"next_offset":q.offset+bytes.len()as u64,"text":String::from_utf8_lossy(&bytes)}),
    ))
}
async fn metrics(State(app): State<App>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    auth(&app, &headers)?;
    let state = operator::status(&app.db, &id).map_err(error)?;
    let plan: Plan = serde_json::from_value(state["plan"].clone()).map_err(error)?;
    let root = plan.workspace.canonicalize().map_err(error)?;
    if !root.starts_with(&app.workspace) {
        return Err(error("workspace outside server root"));
    };
    let mut result = Vec::new();
    for stage in plan.stages {
        if let Some(log) = stage.metric_log {
            let path = root.join(log);
            if let Ok(actual) = path.canonicalize() {
                if !actual.starts_with(&root) {
                    return Err(error("metric path escapes workspace"));
                };
                let file = std::fs::File::open(actual).map_err(error)?;
                let mut bytes = Vec::new();
                file.take(16 * 1024 * 1024)
                    .read_to_end(&mut bytes)
                    .map_err(error)?;
                let text = String::from_utf8_lossy(&bytes);
                let rows: Vec<Value> = text
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
                let end = rows.len();
                let start = end.saturating_sub(200);
                result.push(json!({"stage":stage.id,"integrity":"live_unsealed_telemetry","rows":&rows[start..end]}));
            }
        }
    }
    Ok(Json(
        json!({"run_id":id,"streams":result,"scope":"Live telemetry; acceptance uses separately hash-verified artifacts."}),
    ))
}
pub async fn serve(
    db: String,
    workspace: PathBuf,
    address: std::net::SocketAddr,
    trusted: bool,
) -> Result<()> {
    if !trusted {
        bail!("--trusted-local required")
    };
    if !address.ip().is_loopback() {
        bail!("v1 API binds loopback only; use an authenticated gateway for external deployment")
    }
    let token = std::env::var("AEGIS_OPERATOR_TOKEN").unwrap_or_default();
    if token.len() < 24 {
        bail!("AEGIS_OPERATOR_TOKEN must contain at least 24 characters")
    }
    operator::open(&db)?;
    let profile = std::env::var("AEGIS_RUNTIME_PROFILE")
        .ok()
        .map(|path| -> Result<_> {
            let profile: crate::runtime::Profile = serde_json::from_slice(&std::fs::read(path)?)?;
            profile.validate()?;
            Ok(Arc::new(profile))
        })
        .transpose()?;
    let capacity = std::env::var("AEGIS_API_CAPACITY")
        .ok()
        .map(|path| -> Result<_> {
            if !profile.as_ref().is_some_and(|p| p.require_container) {
                bail!("API capacity requires a container-only runtime profile")
            }
            crate::capacity::Pool::new(serde_json::from_slice(&std::fs::read(path)?)?)
        })
        .transpose()?;
    let language_servers: Vec<crate::language::Server> = std::env::var("AEGIS_LANGUAGE_SERVERS")
        .ok()
        .map(|path| crate::knowledge::read_json(std::path::Path::new(&path)))
        .transpose()?
        .unwrap_or_default();
    if language_servers.len() > 32 {
        bail!("at most 32 language servers")
    }
    let mut language_ids = std::collections::BTreeSet::new();
    for server in &language_servers {
        crate::language::validate(server)?;
        if !language_ids.insert(&server.id) {
            bail!("duplicate language server ID")
        }
    }
    if !language_servers.is_empty() && profile.as_ref().is_some_and(|p| p.require_container) {
        bail!("host language servers cannot be enabled with a container-only profile")
    }
    let model_adapters: Vec<crate::model::Adapter> = std::env::var("AEGIS_MODEL_ADAPTERS")
        .ok()
        .map(|path| crate::knowledge::read_json(std::path::Path::new(&path)))
        .transpose()?
        .unwrap_or_default();
    if model_adapters.len() > 32 {
        bail!("at most 32 model adapters")
    }
    let mut model_ids = std::collections::BTreeSet::new();
    for adapter in &model_adapters {
        crate::model::validate(adapter)?;
        if !model_ids.insert(&adapter.id) {
            bail!("duplicate model adapter ID")
        }
    }
    if !model_adapters.is_empty() && profile.as_ref().is_some_and(|p| p.require_container) {
        bail!("host model adapters cannot be enabled with a container-only profile")
    }
    let editor_assets=match (std::env::var("AEGIS_EDITOR_ASSETS").ok(),std::env::var("AEGIS_EDITOR_ASSETS_MANIFEST_SHA256").ok()) {
        (None,None)=>None,
        (Some(path),Some(hash))=>Some(Arc::new(crate::editor_assets::Assets::load(std::path::Path::new(&path),&hash)?)),
        _=>bail!("editor assets require both AEGIS_EDITOR_ASSETS and AEGIS_EDITOR_ASSETS_MANIFEST_SHA256"),
    };
    let state = App {
        editor_assets,
        model_adapters: Arc::new(model_adapters),
        model_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        language_servers: Arc::new(language_servers),
        language_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        capacity,
        profile,
        db,
        workspace: workspace.canonicalize()?,
        token: Arc::new(token),
        runs: Arc::new(tokio::sync::Semaphore::new(2)),
    };
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../../assets/operator.html")) }),
        )
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","service":"aegis-operator","version":1})) }),
        )
        .route(
            "/workspace",
            get(|| async { Html(include_str!("../../../assets/workspace.html")) }),
        )
        .route("/v1/workspace/tree", get(editor_tree))
        .route("/v1/workspace/file", get(editor_read).post(editor_save))
        .route("/v1/workspace/search", get(editor_search))
        .route("/v1/workspace/git", get(editor_git))
        .route("/editor-assets/{*path}", get(editor_asset))
        .route("/v1/editor/assets", get(editor_asset_status))
        .route("/v1/wire/validate", post(wire_validate))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/language/query", post(language_query))
        .route("/v1/models", get(model_catalog))
        .route("/v1/models/stream", post(model_stream))
        .route("/v1/knowledge/build", post(knowledge_build))
        .route("/v1/knowledge/context", post(knowledge_context))
        .route("/v1/chats/export", post(chat_export))
        .route("/v1/projects/discover", post(discover_project))
        .route("/v1/runs", post(start))
        .route("/v1/runs/{id}", get(run_status))
        .route("/v1/runs/{id}/events", get(events))
        .route("/v1/runs/{id}/control", post(control))
        .route("/v1/runs/{run}/logs/{stage}/{attempt}", get(logs))
        .route("/v1/graph", get(graph))
        .route("/v1/readiness", get(readiness))
        .route("/v1/use-cases", get(use_cases))
        .route("/v1/runtime", get(runtime_profile))
        .route("/v1/runs/{id}/notes", post(note))
        .route("/v1/runs/{id}/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state);
    println!("AEGIS operator listening at http://{address}");
    axum::serve(tokio::net::TcpListener::bind(address).await?, app).await?;
    Ok(())
}
