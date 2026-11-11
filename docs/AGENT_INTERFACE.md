# Human and AI operator contract

These are repository operating instructions, not an installed ChatGPT skill.
An agent tool adapter can expose the native CLI or authenticated HTTP routes.
Run `aegis-data capabilities` or authenticated `GET /v1/capabilities` for the same versioned interface catalogue. It distinguishes implemented routes from unavailable integrations and does not grant execution permission. Use `--help` and `use-cases` for broader CLI operations.

1. Inspect the project, declared inputs, model/codec identities and required tools.
2. Use project-doctor and runtime-check; report unavailable tools as unavailable.
3. Submit only an authorized plan and profile. Keep command/args separate. Host
   execution needs explicit trust; use pinned OCI execution where appropriate.
4. Retain run ID, plan/profile hashes, event cursor and artifact/manifest identities.
5. Follow status/events/logs; distinguish telemetry from sealed acceptance evidence.
6. Pause/cancel using operator controls. Respect retry budgets and checkpoint
   validation; do not force resume merely because a file exists.
7. Rehearse patches against protected tests; promote only through case policy.
8. Export only verified completed runs; retain and verify the manifest anchor.
9. Transfer large model shards through artifact commands, never oversized job RPC.
10. Report measured, failed and untested results separately. Never fabricate scores
    or infer model quality from weight size, passing lint or successful launch.

The API exposes job operations, telemetry, optional knowledge operations and
configured local model streaming/language queries; it does not cover every CLI command. External access needs a properly authenticated
gateway or verified tunnel; do not make the loopback listener public. Credentials
belong in scoped runtime configuration, not plans, logs, chat or evidence bundles.

Model tools must declare supported architecture/format/backend, device
requirements and streaming contract. An AI should select only verified compatible
adapters and preserve confirmation/promotion rules. Unknown weight formats must
fail clearly rather than falling back to unsafe loading.

## Product boundary and optional integrations

AEGIS is the AI-operated backend for data, software execution, evaluation and
evidence. The existing editor is retained as a client; richer human interaction
is a later layer. The user's model generator remains independent and will connect
through an explicit adapter. No external model API key is required by this core.

Bounded lexical document retrieval and reviewed chat JSONL export are available
through [KNOWLEDGE.md](KNOWLEDGE.md). Document-based model training remains a
separate, unavailable adapter integration. Loading documents must never implicitly train a model.
Training candidates require source provenance, contamination checks, held-out
evaluation and accepted checkpoint identities. Training does not guarantee recall.
Local bounded LSP queries and executable model streaming are documented in
[AI_BACKEND.md](AI_BACKEND.md). DAP, generated skills and the durable model-driven
coding loop remain backlog, not features supplied by installing an editor library. AEGIS branding applies to our
product; upstream dependency names and required attribution remain intact.
