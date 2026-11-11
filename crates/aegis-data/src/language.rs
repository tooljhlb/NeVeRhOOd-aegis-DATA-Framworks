//! Bounded one-shot LSP sessions for AI callers. Server binaries are trusted host code.
use crate::rpc::Session;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub schema_version: u32,
    pub id: String,
    pub program: PathBuf,
    pub program_sha256: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub input_files: std::collections::BTreeMap<PathBuf, String>,
    pub language_id: String,
    pub timeout_seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub path: String,
    pub operation: String,
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub character: u32,
}

pub fn validate(server: &Server) -> Result<()> {
    if server.schema_version != 1
        || server.id.is_empty()
        || server.id.len() > 80
        || !server.program.is_absolute()
        || !server.program.is_file()
        || server.args.len() > 64
        || server
            .args
            .iter()
            .any(|s| s.len() > 4096 || s.contains('\0'))
        || server.language_id.is_empty()
        || server.language_id.len() > 80
        || !(1..=120).contains(&server.timeout_seconds)
        || crate::ingest::hash_file(&server.program)? != server.program_sha256
    {
        bail!("invalid language server configuration or executable identity")
    }
    if server.input_files.len() > 32 {
        bail!("at most 32 declared adapter inputs")
    }
    for (path, expected) in &server.input_files {
        if !path.is_absolute() || crate::ingest::hash_file(path)? != *expected {
            bail!("adapter input identity mismatch")
        }
    }
    Ok(())
}

pub fn query(root: &Path, server: &Server, query: Query) -> Result<Value> {
    validate(server)?;
    let root = root.canonicalize()?;
    let source = crate::editor::read(&root, &query.path)?;
    let text = source["text"].as_str().context("missing source")?;
    let (method, capability) = match query.operation.as_str() {
        "diagnostics" => ("textDocument/diagnostic", "diagnosticProvider"),
        "symbols" => ("textDocument/documentSymbol", "documentSymbolProvider"),
        "hover" => ("textDocument/hover", "hoverProvider"),
        "definition" => ("textDocument/definition", "definitionProvider"),
        "references" => ("textDocument/references", "referencesProvider"),
        _ => bail!("operation must be diagnostics, symbols, hover, definition or references"),
    };
    if ["hover", "definition", "references"].contains(&query.operation.as_str()) {
        let line = text
            .split('\n')
            .nth(query.line as usize)
            .context("line outside source")?;
        if query.character as usize > line.trim_end_matches('\r').encode_utf16().count() {
            bail!("UTF-16 character outside source line")
        }
    }
    let uri = reqwest::Url::from_file_path(root.join(&query.path))
        .map_err(|_| anyhow::anyhow!("invalid document URI"))?
        .to_string();
    let root_uri = reqwest::Url::from_directory_path(&root)
        .map_err(|_| anyhow::anyhow!("invalid root URI"))?
        .to_string();
    let mut session = Session::spawn(
        &root,
        &server.program,
        &server.args,
        server.timeout_seconds,
        None,
    )?;
    session.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":std::process::id(),"rootUri":root_uri,"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"configuration":true,"applyEdit":false},"textDocument":{"diagnostic":{},"publishDiagnostics":{"versionSupport":true}}},"workspaceFolders":[{"uri":root_uri,"name":"aegis-workspace"}]}}))?;
    let initialized = session.response(1)?;
    if initialized["capabilities"]
        .get("positionEncoding")
        .is_some_and(|v| v != "utf-16")
    {
        bail!("server selected unsupported position encoding")
    }
    session.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))?;
    session.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":server.language_id,"version":1,"text":text}}}))?;
    let advertised = initialized["capabilities"]
        .get(capability)
        .is_some_and(|v| !v.is_null() && v != false);
    let result = if query.operation == "diagnostics" && !advertised {
        loop {
            let v = session.receive()?;
            if session.respond(&v)? {
                continue;
            }
            if v["method"] == "textDocument/publishDiagnostics"
                && v["params"]["uri"] == uri
                && (v["params"].get("version").is_none() || v["params"]["version"] == 1)
            {
                break json!({"kind":"full","items":v["params"]["diagnostics"],"transport":"publishDiagnostics"});
            }
        }
    } else {
        if !advertised {
            bail!("server does not advertise requested capability")
        }
        session.send(json!({"jsonrpc":"2.0","id":2,"method":method,"params":{"textDocument":{"uri":uri},"position":{"line":query.line,"character":query.character},"context":{"includeDeclaration":true}}}))?;
        session.response(2)?
    };
    session.send(json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}))?;
    session.response(3)?;
    session.send(json!({"jsonrpc":"2.0","method":"exit"}))?;
    if crate::editor::read(&root, &query.path)?["sha256"] != source["sha256"] {
        bail!("source changed during language analysis")
    }
    validate(server)?;
    Ok(
        json!({"schema_version":1,"server_id":server.id,"server_config_sha256":crate::ingest::hash(&serde_json::to_vec(server)?),"program_sha256":server.program_sha256,"source_sha256":source["sha256"],"path":query.path,"operation":query.operation,"position_encoding":"utf-16","server_info":initialized["serverInfo"],"capabilities":initialized["capabilities"],"result":result,"result_sha256":crate::ingest::hash(&serde_json::to_vec(&result)?),"edits_applied":false,"scope":"trusted local server; bounded one-shot analysis, not host isolation or whole-project immutability"}),
    )
}
