//! Versioned local model process adapter. Tool calls are proposals, never execution.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Adapter {
    pub schema_version: u32,
    pub id: String,
    pub model_id: String,
    pub model_revision: String,
    pub program: PathBuf,
    pub program_sha256: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub input_files: std::collections::BTreeMap<PathBuf, String>,
    pub timeout_seconds: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub messages: Vec<crate::knowledge::Message>,
    pub max_output_tokens: u32,
    #[serde(default)]
    pub tools: Vec<Tool>,
    pub knowledge: Option<crate::knowledge::ContextRequest>,
}

pub fn validate(adapter: &Adapter) -> Result<()> {
    if adapter.model_id.trim().is_empty()
        || adapter.model_id.len() > 256
        || adapter.model_revision.trim().is_empty()
        || adapter.model_revision.len() > 512
    {
        bail!("model identity and revision required")
    }
    crate::language::validate(&crate::language::Server {
        schema_version: adapter.schema_version,
        id: adapter.id.clone(),
        program: adapter.program.clone(),
        program_sha256: adapter.program_sha256.clone(),
        args: adapter.args.clone(),
        input_files: adapter.input_files.clone(),
        language_id: "aegis-model-adapter".into(),
        timeout_seconds: adapter.timeout_seconds,
    })
}

pub fn generate(
    root: &Path,
    adapter: &Adapter,
    request: Request,
    cancel: Arc<AtomicBool>,
    mut emit: impl FnMut(Value) -> Result<()>,
) -> Result<Value> {
    validate(adapter)?;
    if cancel.load(Ordering::Relaxed) {
        bail!("model request cancelled")
    }
    if request.messages.is_empty()
        || request.messages.len() > 128
        || !(1..=8192).contains(&request.max_output_tokens)
        || request.tools.len() > 32
    {
        bail!("invalid model request bounds")
    }
    let mut names = BTreeSet::new();
    for tool in &request.tools {
        if tool.name.is_empty()
            || tool.name.len() > 80
            || !tool
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            || !names.insert(tool.name.clone())
            || tool.description.len() > 4096
            || !tool.input_schema.is_object()
        {
            bail!("invalid tool declaration")
        }
    }
    if request.messages.iter().any(|m| {
        !["system", "user", "assistant", "tool"].contains(&m.role.as_str())
            || m.content.is_empty()
            || m.content.contains('\0')
    }) || serde_json::to_vec(&request)?.len() > 256 * 1024
    {
        bail!("invalid messages or model request exceeds 256 KiB")
    }
    let request_hash = crate::ingest::hash(&serde_json::to_vec(&request)?);
    let knowledge = request
        .knowledge
        .map(crate::knowledge::context)
        .transpose()?;
    let mut session = crate::rpc::Session::spawn(
        root,
        &adapter.program,
        &adapter.args,
        adapter.timeout_seconds,
        Some(cancel.clone()),
    )?;
    session.send(json!({"jsonrpc":"2.0","id":1,"method":"aegis.initialize","params":{"protocol_version":1,"model_id":adapter.model_id,"model_revision":adapter.model_revision}}))?;
    let hello = session.response(1)?;
    if hello["protocol_version"] != 1
        || hello["model_id"] != adapter.model_id
        || hello["model_revision"] != adapter.model_revision
        || hello["capabilities"]["streaming"] != true
        || (!request.tools.is_empty() && hello["capabilities"]["tool_calls"] != true)
    {
        bail!("model adapter identity or capability mismatch")
    }
    emit(
        json!({"type":"started","model_id":adapter.model_id,"model_revision":adapter.model_revision,"request_sha256":request_hash,"adapter_config_sha256":crate::ingest::hash(&serde_json::to_vec(adapter)?)}),
    )?;
    session.send(json!({"jsonrpc":"2.0","id":2,"method":"aegis.generate","params":{"messages":request.messages,"max_output_tokens":request.max_output_tokens,"tools":request.tools,"knowledge":knowledge}}))?;
    let mut text = String::new();
    let mut count = 0;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("model request cancelled")
        }
        let v = session.receive()?;
        if v.get("method").is_some() && v.get("id").is_some() {
            session.send(json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"model adapters cannot execute client methods"}}))?;
            continue;
        }
        if v["method"] == "aegis.delta" {
            if v["params"]["request_id"] != 2 {
                bail!("delta belongs to another request")
            }
            let delta = v["params"]["text"].as_str().context("missing text delta")?;
            count += 1;
            if count > 1024 || text.len().saturating_add(delta.len()) > 512 * 1024 {
                bail!("generation output exceeds limits")
            }
            text.push_str(delta);
            emit(json!({"type":"delta","sequence":count,"text":delta}))?;
        } else if v["id"] == 2 && v.get("method").is_none() {
            if v.get("error").is_some() {
                bail!("model generation failed: {}", v["error"])
            }
            break v
                .get("result")
                .cloned()
                .context("missing generation result")?;
        } else {
            bail!("unexpected model adapter message")
        }
    };
    if result["text"].as_str() != Some(text.as_str())
        || !["stop", "length", "tool_calls"]
            .contains(&result["finish_reason"].as_str().unwrap_or(""))
    {
        bail!("final response does not match streamed text or finish reason")
    }
    let calls = result["tool_calls"]
        .as_array()
        .context("tool_calls array required")?;
    if calls.len() > 32 {
        bail!("too many tool proposals")
    }
    let mut ids = BTreeSet::new();
    for call in calls {
        let id = call["id"].as_str().context("tool call ID required")?;
        if id.is_empty()
            || id.len() > 128
            || !ids.insert(id)
            || !names.contains(call["name"].as_str().unwrap_or(""))
            || !call["arguments"].is_object()
        {
            bail!("undeclared or malformed tool proposal")
        }
    }
    if (result["finish_reason"] == "tool_calls") != !calls.is_empty() {
        bail!("finish reason and tool calls disagree")
    }
    validate(adapter)?;
    let receipt = json!({"schema_version":1,"model_id":adapter.model_id,"model_revision":adapter.model_revision,"identity_scope":"model identity asserted by configured adapter; executable hash verified, weights not independently loaded or attested","program_sha256":adapter.program_sha256,"adapter_config_sha256":crate::ingest::hash(&serde_json::to_vec(adapter)?),"request_sha256":request_hash,"result_sha256":crate::ingest::hash(&serde_json::to_vec(&result)?),"result":result,"tool_calls_executed":false,"weights_modified":false});
    emit(json!({"type":"completed","receipt":receipt}))?;
    Ok(receipt)
}
