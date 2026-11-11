# AI-operated backend: local language and model adapters

The framework is an independent tool backend. An AI building a model, a human,
or a separate compute controller can invoke its CLI/API. It does not replace the
user's generator, provision their machines, import their credentials or change
running external jobs. No hosted model or provider key is required. Model names
are operator-configured; the product is AEGIS, while upstream dependencies retain
accurate names and attribution.

## Local language servers

A server JSON configuration contains:

```json
{"schema_version":1,"id":"python-tools","program":"/absolute/path/to/ruff","program_sha256":"ACTUAL_SHA256","args":["server"],"language_id":"python","timeout_seconds":30,"input_files":{}}
```

Use `sha256sum` to obtain executable hashes. For interpreted servers, declare the
absolute script and relevant configuration paths in `input_files` with hashes.
Those declared inputs and the executable are checked before and after a query.
This does not attest undeclared dependencies, all project configuration, or a file
changed and restored between checks. Keep the workspace and adapter installation
under exclusive trusted control while analyzing it.

```sh
aegis-data language-query /repository server.json query.json result.json --trusted-local
```

Query JSON: `{"path":"src/example.py","operation":"diagnostics"}`. Other
operations are `symbols`, `hover`, `definition`, `references`. Position operations
also take zero-based `line` and `character` (UTF-16 code units). The server must
advertise the requested capability. Diagnostics use pull when advertised, otherwise
wait for a matching publish notification. Missing capability, timeout, malformed
protocol, changed source or identity mismatch fails instead of returning success.

The bridge follows LSP initialization, document open, request, shutdown and exit.
It returns raw structured results, source/config/executable identities, capabilities
and a result hash. It never applies server edits or runs server-proposed commands.
Document reads inherit editor boundaries: relative paths, no symlinks or known
secret paths, regular UTF-8 files up to 128 KiB. Results can reference other files;
those references do not authorize subsequent reads or edits. Rename, completion,
code-action application, DAP and persistent LSP sessions remain backlog.

For API use, `AEGIS_LANGUAGE_SERVERS=/absolute/servers.json` points to an array of
these configurations supplied by the operator at server startup. POST:

```json
{"server_id":"python-tools","query":{"path":"src/example.py","operation":"diagnostics"}}
```

to `/v1/language/query`. Unknown servers are rejected. API users cannot supply a
program, command arguments or arbitrary workspace root. Two language queries may
run concurrently per API process; saturation returns 429. Environment configuration
is optional: without it no language server can be executed through this route.

## Local model adapter protocol v1

This is an executable transport contract, not a universal weights loader. A model
runtime or generator supplies a compatible local wrapper. The wrapper can connect
to an existing locally running model to avoid reloading large weights every call.
The framework launches one adapter process per request; it does not install or
bundle an inference engine or a pretrained model.

Adapter JSON:

```json
{"schema_version":1,"id":"aegis-local","model_id":"aegis","model_revision":"your-checkpoint-manifest-hash","program":"/absolute/path/to/adapter","program_sha256":"ACTUAL_SHA256","args":[],"input_files":{},"timeout_seconds":120}
```

Declare interpreted scripts and configuration assets in `input_files`. Executable
and declared file hashes are verified before and after generation. Model identity
and revision are checked against the adapter handshake but remain assertions of
that adapter: this framework does not independently attest its loaded weights.
Avoid declaring huge model bodies as per-request adapter files; supply their
immutable checkpoint/manifest identity in `model_revision` and validate weight
loading in the runtime integration.

Request JSON:

```json
{"messages":[{"role":"user","content":"Inspect this proposal"}],"max_output_tokens":1024,"tools":[],"knowledge":null}
```

Optional `knowledge` takes the request format in [KNOWLEDGE.md](KNOWLEDGE.md), so
retrieval can be enabled per model call or omitted. The generated context remains
a separate field, not privileged system text. Adapter authors must preserve that
separation. A model request including inline knowledge is bounded to 256 KiB.
Messages use role/content pairs; tool-result linkage and richer multimodal messages
need future protocol extensions. No automatic chat collection or training occurs.

```sh
aegis-data model-run /repository adapter.json request.json NEW_RUN_DIRECTORY --trusted-local
```

The new directory contains live `events.jsonl`. Only validated completion publishes
`receipt.json`. Failures append a failed event and return a nonzero exit code.
There is no restart-from-token or cached replay contract for generation yet.

The adapter communicates on stdin/stdout using UTF-8 JSON-RPC 2.0 framed as
`Content-Length: BYTE_COUNT\r\n\r\nJSON`. Diagnostics must not be printed to stdout.
The host currently discards adapter stderr; add runtime-specific logging outside
this transport when needed.

1. Host request ID 1, `aegis.initialize`, parameters `protocol_version:1`,
   `model_id`, `model_revision`. Result must echo those identities and include
   `capabilities:{streaming:true,tool_calls:true|false}`.
2. Host request ID 2, `aegis.generate`, parameters `messages`, `max_output_tokens`,
   `tools`, and optional prepared `knowledge`.
3. Adapter notifications `aegis.delta` contain `params:{request_id:2,text:"..."}`.
4. Final ID 2 response has `result:{text:"concatenated deltas",tool_calls:[],
   finish_reason:"stop"|"length"|"tool_calls"}`. Mismatched text is rejected.
5. A tool proposal contains `id`, declared tool `name`, and object `arguments`.
   The host checks names, unique IDs and structural bounds; a downstream tool
   executor must validate its full schema and policy. Nothing is executed here.
   Adapter-to-host requests during generation are denied.

Bounds: 128 messages, 32 declared/proposed tools, 8192 requested output tokens,
1024 deltas, 512 KiB accumulated text, 1 MiB/frame, 8 MiB session, 2048 inbound
frames, 1..120 seconds total process/session deadline. Token counts are enforced
by the adapter; host output limits are byte/event limits, not tokenizer metrics.
The frame reader uses a bounded channel. Hung sessions and normal cleanup kill
and reap the child process group on Unix. Linux also sets parent-death handling.
Escaped/daemonized descendants, host CPU/memory/disk isolation and Windows process
supervision are not certified. These adapters execute trusted local code and may
read/write/network with the host account; they are not untrusted plugin sandboxes.

## Streaming API and compute integration

Set `AEGIS_MODEL_ADAPTERS=/absolute/adapters.json` to an array of configurations at
server startup. GET `/v1/models` lists configured adapter/model IDs and revisions.
POST `/v1/models/stream` with `{"adapter_id":"aegis-local","request":{...}}`
returns SSE `data:` JSON events: `started`, `delta`, `completed` or `failed`.
Only `completed` with its validated receipt represents success. EOF without it is
incomplete. Invalid generation requests may produce a failed event after HTTP 200;
clients must inspect events, not only HTTP status. This is AEGIS v1, not a claim of
OpenAI-compatible routes or compatibility with every chat UI.

Two model calls may run concurrently per API process. Events use a bounded queue
and wait for consumption within the deadline; slow/disconnected consumers cannot
retain the child indefinitely. Closing the SSE stream cancels and reaps the
adapter. This does not prove a remote service contacted by that adapter stopped
its own generation; the wrapper must forward cancellation and bound its requests.

All data routes require the existing bearer token and loopback binding. Do not
embed tokens in plans, skills or chat. Host language/model adapters cannot be
enabled under a container-only runtime profile. Their two slot limits are separate
from operator capacity, not a fleet scheduler or GPU memory reservation. Running
heavy models alongside training needs admission in the external compute runtime.

Your AI/compute controller can use `/v1/capabilities`, these model/language routes,
workspace operations and existing run/worker/evidence contracts. Tool proposals
must pass those contracts' policy checks. There is no automatic connection to your
existing generator or machines until their concrete adapter is installed.

## Evidence and remaining loop

`verify_agent_interfaces.py` exercises a real Ruff server (including F401/F821),
CLI/API parity, API boundaries, deterministic model fixture streaming, malformed
final/tool rejection, deadlines and cancellation on disconnect. The Python
`fixtures/adapters/contract_model.py` is test infrastructure, explicitly NOT a
model or production inference backend. These checks do not measure Aegis model
quality or prove its generator integration.

The next acceptance milestone is a persistent model-driven task: isolated Git
worktree, source/diagnostic context, proposed patch, protected tests, multi-file
review, promotion/rollback and recovery after controller loss. Operator jobs and
case rehearsals exist today, but a unified autonomous task lifecycle, task worktree
manager and multi-file atomic promotion are not implemented by this adapter change.
Optional local Monaco UI with saved-file diagnostics is now documented under
`subsystems/editor/README.md`. Persistent monaco-languageclient integration, DAP,
generated skills, MCP/extension loading,
browser agent control and installation/outcome matrices remain separate work.
