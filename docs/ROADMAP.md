# Executable system backlog

Each task needs a runnable command, output artifact, failure behavior and an evidence receipt. More frameworks alone do not complete the system.

| Priority | Deliverable | Acceptance requirement |
|---|---|---|
| P0 | Codec registry and round trips | Encode/decode real audio/video; record codec hash, shape/scaling, reconstruction and timing metrics |
| P0 | Protobuf/FlatBuffers batch bridge | Text Protobuf → NPY → trainer works. Extend to aligned audio/video FlatBuffers; require identical tensor values; corrupted/truncated inputs fail; enforce token/byte budgets |
| P0 | Data registry and split manifests | Immutable upstream revisions, license/media rights, per-shard hashes, group/temporal split isolation, fuzzy dedup and benchmark contamination audit |
| P0 | Held-out trainer validation | Real small corpus; fixed train/validation manifests; baseline/final held-out losses; no selection on test set |
| P1 | Native attention and backward | Finite differences, CPU oracle parity, causal/media masking tests, memory usage and measured GPU throughput |
| P1 | Optimizer/scheduler/state | Correct accumulation, gradient clipping, BF16/FP32 state handling, overflow policy and scheduler resume |
| P1 | Pinned data queue | Double/triple buffer ownership, cancellation, backpressure, safe stream/event dependencies and transfer/compute overlap trace |
| P1 | Durable distributed checkpoint | Atomic manifest publish, all tensor/optimizer/RNG/cursor hashes, interruption injection and exact CPU / tolerance-defined GPU resume |
| P1 | Official benchmark workers | SWE-bench isolation; licensed exam sets; pinned VBench/MM-Vet/FAD engine and artifact provenance; error/unrun states never count as pass |
| P1 | Factuality/RAG judge | Local model endpoint, claim-support evaluation, prompt injection tests, human calibration, abstention and disagreement reporting |
| P2 | Remote worker scaling | Authenticated transport, heartbeats, fencing, bounded transfer, retries, cancellation and OCI execution are implemented and tested. Remaining: multi-controller coordination, durable shared quotas, independent-host deployment and provider provisioning |
| P2 | NCCL distributed training | Independently allocated GPU workers, collective timeout handling, matched baseline parity, rank failure/recovery and world-size change policy |
| P2 | Training observability | Tokens/sec, samples/sec, step latency breakdown, VRAM, host memory, data wait, gradients and modality contribution |
| P2 | Serving and artifact registry | Local executable model contract has authenticated SSE, declared IDs, bounded queues and disconnect cancellation. Remaining: actual runtime adapters, weight validation, durable chat/reconnect, fleet admission, HF immutable uploads, readiness and rollout/rollback |
| P3 | Scale and production selection | Equal-budget ablations, real held-out gains, cost/quality frontier, robustness and human review |

Data acquisition, GPU provisioning and model quality are separate from source-code readiness. Do not measure readiness by parameter bytes, framework counts, or an arbitrary percentage.

## Implemented follow-up slice

The repository now includes polyglot code inventory/check automation, artifact-derived release gates, per-shard manifest validation, comparable experiment receipts, local lease renewal/cancellation, and a bounded real-source heldout/reference experiment. Production codec integration, benchmark decontamination, official executable benchmark workers, multi-controller scheduling, independent-host deployment, and native GPU training remain open. Remote retry/cancellation and digest-pinned OCI execution are implemented.


Filesystem-store chunk transport now handles artifacts beyond the worker RPC/export bounds. HTTP/object-storage adapters, checkpoint tensor semantics and cross-host deployment remain open. Practical outcome reports are separate from the acceptance registry. See [capabilities](CAPABILITIES.md).
