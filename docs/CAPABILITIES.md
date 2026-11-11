# Current capabilities, interfaces and next integrations

AEGIS DATA is a Rust runtime for data preparation, evaluation evidence and
bounded project/model workflows. C++/CUDA reference primitives and Python
research/verification adapters are separate components. A CLI command is an
interface to a capability, not a separate engine. The HTTP API and cockpit
expose selected operations from the same native core; not every CLI command has
an HTTP route. `main.rs` dispatches commands; the library modules implement them.
Humans and AI operators can use the same contracts and policy gates.

## Implemented capability map

| Capability | Current behavior and boundary |
|---|---|
| Data import | Provenance/license/revision-aware JSONL import; exact code formatting preservation; explicit prose normalization |
| Dataset preparation | Group-based splits, exact duplicate cross-split rejection, Protobuf records, immutable shard manifests and hash validation |
| Token export | Deterministic bounded document-local byte-token sampling with source/tokenizer/group/offset sidecars |
| Statistical evaluation | Classification/regression metrics, text metrics and artifact-derived acceptance gates; missing evidence fails |
| Experiment comparison | Model/data/evaluator/decoding identity comparisons and confounder reporting |
| Prediction adapters | Bounded executable prediction contract, exact coverage validation, paired exact/regression comparison and group bootstrap |
| Research training | Small CPU/reference text and latent multimodal experiments, held-out loss and exact CPU checkpoint restore; not a frontier trained model |
| Code checks | Polyglot tool registry, source inventory, bounded lint/format/test execution, explicit unavailable-tool results; registered profiles are not all independently certified |
| Durable operator | DAG stages, bounded concurrency, attempts, timeouts, logs, pause/cancel, run identity checks and crash recovery |
| Runtime profiles | Allowed programs/images, retry/time limits and conservative per-run container resource envelopes |
| API capacity | Atomic CPU/memory reservations across active runs in one API process; not global fleet reservations |
| Local/remote workers | Leases, heartbeats, fencing, authenticated chunk transfer, cancellation, verified returned artifacts and targeted worker identity |
| Isolation | Actual digest-pinned OCI execution, no network, reduced privileges and CPU/memory/PID limits; not a general VM security certification |
| Repair/upgrade cases | Protected tests, isolated before/after checks, executable proposal adapters, hash-bound acceptance, bounded single-file promotion and rollback |
| Provenance graph | Append-only event chain, graph projections, dependency impact and quarantine checks; declared graph catalogue entries are not separate implemented engines |
| Compute admission | Filter caller-supplied resource/capability/image/cost inventory; no automatic provisioning or billing attestation |
| Incident rehearsal | Contained worker/controller failure and disconnect drills; not authorized production chaos execution |
| Reproducibility | Portable case packages and replay, revision-scoped receipts, manifest validation and evidence-backed readiness registry |
| Project workflow | Doctor, profile-governed build/tests, failure withholding, corrected runs, bounded export and externally anchored export verification |
| Installation | Tested Linux x86_64 release package, fixed-member checksums, fresh-directory install and retained-version rollback |
| Operating UI | Authenticated loopback API, cockpit, run status/events/logs/metrics/control, use-case catalogue and readiness view |
| Large artifacts | Separate filesystem-store, hash-addressed chunk transport and resumed materialization; see ARTIFACTS.md |
| Practical outcomes | Sample-counted first-success/recovery latency, unnecessary reruns, regression rate and operator interventions from recorded observations; see OUTCOMES.md |

Historical receipts certify their recorded revisions. New code needs new tests.
The evidence index points to the latest fully verified milestone and preserves
older receipts. The acceptance registry is a backlog, not a usefulness rating.

## Models and weight types

You can build different AEGIS models and use this system to orchestrate their
preparation, training commands, evaluation, artifact identity and recovery. The
runtime does not currently understand or execute every weight format itself.
A 5 GB shard is a file object, not a model-format definition. Artifact transfer
preserves its bytes; it does not validate tensor compatibility or load the model.

The next model integration contract should bind:

- Stable model ID/version, architecture/configuration and weight format/version.
- Ordered shard manifest, hashes, tensor names/shapes/dtypes and index identity.
- Tokenizer, chat template, context limits, modality codecs and processor hashes.
- Quantization/compression method, precision/scaling and required loader backend.
- Runtime ABI, supported devices, memory estimate and executable launch profile.
- Checkpoint objective/optimizer/RNG/sampler identities when training is resumed.
- Health/readiness checks, benchmark receipts, license and provenance.

Support needs tested adapters for each format/backend (for example safetensors,
GGUF, framework checkpoints or custom AEGIS weights). These are planned adapter
categories, not a claim that all these loaders exist in this repository.
Compatibility must be checked before launch; byte size alone cannot establish it.

## AI operators and inline chat

AI can already call the CLI or authenticated operator API and consume JSON,
logs, status and receipts. The cockpit exposes operator telemetry; this is not
LLM token streaming. The proposed agent operating instructions are documented
in AGENT_INTERFACE.md. A chat model can drive this contract through a tool adapter.

Configured local executable model adapters now expose a versioned handshake,
CLI event logs, authenticated SSE streaming, model identity discovery, bounded
queues, output validation and disconnect cancellation. Local one-shot LSP queries
expose structured diagnostics and navigation. See [AI_BACKEND.md](AI_BACKEND.md)
for contracts, evidence and limits. A compatible runtime wrapper is still required;
these features are not a universal weights loader or hosted inference service.
Remaining inline-chat work includes durable conversation/tool-result correlation,
reconnect/resume, qualified agent permissions and the unified coding-task loop.
Keep inference streams separate from durable job events. Attaching an AI does not
bypass trust, protected tests or evidence-based promotion.

## Useful extensions built on these contracts

Change previews and compatibility matrices; performance regression investigation;
model-format conversion validation; architecture/loss ablation tournaments;
release evidence bundles; dataset retirement impact; training curriculum selection;
workflow reuse across projects; qualified AI operator permissions; cost/quality
routing; and model fleet health. Each requires its own executable adapter,
acceptance tests and observed evidence before it becomes a verified capability.
