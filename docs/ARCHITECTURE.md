# Architecture

AEGIS DATA Frameworks is an independent developer automation, evaluation and evidence runtime. It is not coupled to any private model, cluster, notebook service, or hosted provider.

## Main execution path

1. **Discover/ingest:** inspect trusted project manifests, prepare source/data and record provenance.
2. **Validate:** enforce immutable manifests, content IDs, split/group constraints and input budgets.
3. **Schedule:** use durable SQLite-backed stage dependencies, bounded execution, retries, leases and fencing.
4. **Execute:** invoke local trusted tools or authenticated remote workers with appropriate isolation policies.
5. **Measure:** compute typed metrics, compare candidate and baseline identities, record logs and append-only events.
6. **Decide:** evaluate evidence-backed gates; mark failures and missing proofs instead of guessing.
7. **Export:** publish hash-anchored receipts and artifacts; provide separately chunked transport for larger files.

## Components

| Layer | Implementation |
| --- | --- |
| CLI and library | Rust/Tokio with versioned entry points |
| Durable control | SQLite WAL, leases, run IDs, retry/cancel semantics |
| Data/interop | JSONL input, framed Protobuf records; optional FlatBuffers schema |
| Native numeric reference | C++17 and optional isolated CUDA primitives |
| Research examples | Python/PyTorch CPU reference and verification |
| UI | Authenticated local API, cockpit, source editor and optional Monaco |
| Execution policy | Trusted-local explicit flag or reviewed isolated container runtime |
| Artifact plane | Hash-addressed chunks, manifests, resumable materialization |

## Boundaries and acceptance

A declared capability, graph definition or adapter does not mean an external model or service is running. The reference trainer does not establish frontier-scale training or multimodal output quality. Hashes establish identity and tamper evidence, not statistical validity or scientific generalization.

External schedulers and AI agents can use the same versioned contracts without importing internal credentials or infrastructure. Verify any host command or remote plan before authorizing execution.

See [current capabilities](CAPABILITIES.md), [remote worker transport](REMOTE.md), [project workflow](PRODUCT_LOOP.md), and [roadmap](ROADMAP.md).
