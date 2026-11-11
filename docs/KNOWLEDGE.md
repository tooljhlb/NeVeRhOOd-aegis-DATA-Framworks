# Optional knowledge and training-data interchange

Retrieval is selected per request. Omitting `index` disables it. These native
operations do not invoke a model, require model-provider keys or start training.
Your generator or another model can consume their output through a separate
adapter. Local executable model streaming is available through the separate adapter in
[AI_BACKEND.md](AI_BACKEND.md); an autonomous coding loop remains unimplemented.

## Documents and retrieval

`aegis-data knowledge-build documents.json index.json` takes a JSON array:

```json
[{"source":"local/manual.md","revision":"r1","license":"owned","group_id":"manual-family","text":"Original document text."}]
```

Retain the printed `chunks_sha256` in trusted configuration. Chunks preserve
UTF-8 text and source byte offsets, document and chunk hashes, revisions, licenses
and groups. Limits: 1024 documents, 4096 chunks, 4096 bytes/chunk, 16 MiB text.
This is a bounded lexical index, not a distributed semantic search service.
PDF/Office parsing, embeddings, OCR and incremental indexing are not included.
Do not mix documents with incompatible access permissions in one exposed index.

`aegis-data knowledge-context request.json context.json` takes:

```json
{"prompt":"Question","index":null,"expected_chunks_sha256":null,"top_k":4}
```

That request disables retrieval. To enable it, replace `index` with the complete
index object and supply its trusted `expected_chunks_sha256`. Ranking uses unique
query-term overlap with deterministic ties, not BM25 or semantic similarity.
No matches means no citations. `top_k` is 1..16; prompt limit is 32768 UTF-8 bytes.
The result separates the original prompt and citations, which include text,
provenance and offsets. Model adapters must treat retrieved text as untrusted
reference data, never privileged instructions or permission to execute tools.
The context policy is guidance, not an enforcement mechanism against injection.
A hash supplied by an untrusted source is not independent proof of trust.

## Reviewed chat export

`aegis-data chat-export chats.json training.jsonl` takes a JSON array:

```json
[{"id":"chat-1","group_id":"session-family","source":"local/chat-1","revision":"review-1","license":"owned","approved_for_training":true,"messages":[{"role":"user","content":"Question"},{"role":"assistant","content":"Reviewed answer"}]}]
```

Only explicitly approved conversations are exported. All-unapproved input fails
without publishing output. Unknown fields, malformed messages, duplicate approved
IDs and identical messages assigned to different groups are rejected. Identical
messages in the same group are deduplicated. Approval, license and source fields
are caller assertions, not independent rights or correctness verification.
Remove secrets/private data and verify answers before approval. Retrieval context
is not silently copied into training messages; include only intentionally reviewed
material. API calls do not automatically collect or retain chat histories.

Each JSONL row retains messages, source, revision, license, hashes and a deterministic
group-based train/validation/test label (approximately 90/5/5 across many groups).
Related chats must share group IDs. Near duplicates and benchmark contamination
need separate checks. Small exports may lack held-out groups. Trainer adapters
must respect split labels and supply their own tokenizer, chat template, loss masks,
architecture and optimizer. This generic interchange is not a guarantee that every
trainer can consume it unchanged. Training does not guarantee recall or eliminate
future retrieval. Export never updates weights or starts a training process.

CLI input files and output chat JSONL are bounded to 16 MiB. Outputs use synchronized
same-directory temporary files and hard-link publication without overwriting an
existing path. Concurrent identical publications may fail safely; hard crashes may
leave `.aegis-export-*` temporary files, removable while idle. No retention daemon.

## API and evidence

Bearer-authenticated routes use the same implementations:

- `POST /v1/knowledge/build`: document array; returns index.
- `POST /v1/knowledge/context`: context request; returns cited context.
- `POST /v1/chats/export`: reviewed chats; returns JSONL, SHA256 and training=false.

API retains its existing 1 MiB request limit, including inline index and JSON
escaping. Use CLI for larger inputs. Calls return data without storing corpora.
Model calls use the separately configured `/v1/models/stream` adapter endpoint. `capabilities` advertises these operations.

`verify_knowledge.py` exercises CLI/API parity, retrieval off/on, source preservation,
authentication, corrupted index rejection and training approval. These fixtures
prove plumbing, not retrieval relevance, privacy certification or model quality.
