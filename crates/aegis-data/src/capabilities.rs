//! Discovery describes interfaces, never grants execution authority.
use serde_json::{json, Value};

pub fn catalog() -> Value {
    json!({
        "schema_version": 1,
        "service": "aegis-data",
        "package_version": env!("CARGO_PKG_VERSION"),
        "purpose": "AI-operated data, software execution, evaluation and evidence backend",
        "external_model_api_key_required": false,
        "authorization": "Discovery is not admission. Server profile, workspace boundaries and per-operation trust remain authoritative.",
        "interfaces": [
            {"id":"editor.assets","method":"GET","path":"/v1/editor/assets","effect":"read_optional_frontend_identity","limits":"operator-configured local hash-pinned Monaco bundle; no CDN"},
            {"id":"wire.validate","method":"POST","path":"/v1/wire/validate","effect":"validate_binary_packet","limits":"Protobuf envelope with raw 32-byte content ID; opaque payload schema not verified; API 1 MiB bound"},
            {"id":"language.query","method":"POST","path":"/v1/language/query","cli":"language-query","effect":"execute_trusted_language_server","limits":"operator-registered local servers only; bounded one-shot diagnostics, symbols, hover, definitions and references; no edit application"},
            {"id":"model.catalog","method":"GET","path":"/v1/models","effect":"read_configured_model_identities"},
            {"id":"model.stream","method":"POST","path":"/v1/models/stream","cli":"model-run","effect":"execute_trusted_model_adapter","limits":"local JSON-RPC adapter contract v1; SSE events, disconnect cancellation; proposals only, no tool execution; requires a compatible adapter, not a universal weights loader"},
            {"id":"knowledge.build","method":"POST","path":"/v1/knowledge/build","cli":"knowledge-build","effect":"prepare_optional_context","limits":"UTF-8 text with provenance; lexical index, no embeddings or model"},
            {"id":"knowledge.context","method":"POST","path":"/v1/knowledge/context","cli":"knowledge-context","effect":"prepare_optional_context","limits":"omit index to disable retrieval; no model invocation; index hash required when enabled"},
            {"id":"chat.export","method":"POST","path":"/v1/chats/export","cli":"chat-export","effect":"export_reviewed_data","limits":"caller-approved messages only; no training; generic JSONL requires trainer-specific adapter"},
            {"id":"project.discover","method":"POST","path":"/v1/projects/discover","cli":"project-discover","effect":"read_workspace","limits":"bounded scan; generates a reviewable plan; does not install dependencies"},
            {"id":"workspace.tree","method":"GET","path":"/v1/workspace/tree","cli":"editor-tree","effect":"read_workspace"},
            {"id":"workspace.read","method":"GET","path":"/v1/workspace/file","cli":"editor-read","effect":"read_workspace"},
            {"id":"workspace.search","method":"GET","path":"/v1/workspace/search","effect":"read_workspace","limits":"literal search; not semantic retrieval"},
            {"id":"workspace.git","method":"GET","path":"/v1/workspace/git","effect":"execute_git","limits":"bounded status/diff; no history or merge API"},
            {"id":"workspace.replace","method":"POST","path":"/v1/workspace/file","cli":"editor-save","effect":"write_workspace","limits":"expected SHA256 required; single-file advisory locking; owner editing, not protected-test promotion"},
            {"id":"workspace.restore","method":"POST","path":"/v1/workspace/file","cli":"editor-rollback","effect":"write_workspace","limits":"expected SHA256 and stored snapshot required"},
            {"id":"run.submit","method":"POST","path":"/v1/runs","effect":"execute_plan","limits":"subject to configured admission profile and capacity; host execution is not a sandbox"},
            {"id":"run.status","method":"GET","path":"/v1/runs/{id}","effect":"read_evidence"},
            {"id":"run.events","method":"GET","path":"/v1/runs/{id}/events","effect":"read_evidence","limits":"cursor polling; not chat token streaming"},
            {"id":"run.logs","method":"GET","path":"/v1/runs/{run}/logs/{stage}/{attempt}","effect":"read_evidence","limits":"bounded capture"},
            {"id":"run.control","method":"POST","path":"/v1/runs/{id}/control","effect":"control_run"}
        ],
        "optional_integration_backlog": [
            {"id":"documents.training","status":"not_implemented","requirement":"separate generator adapter; provenance, held-out evaluation and checkpoint acceptance"},
            {"id":"debug.adapter","status":"not_implemented","requirement":"DAP lifecycle, bounded access and cancellation"},
            {"id":"agent.durable_loop","status":"not_implemented","requirement":"model-driven task state, isolated edits, checks and verified promotion"}
        ]
    })
}
