# Coverage against the supplied specification

| Requirement | Current coverage | Missing |
|---|---|---|
| Binary/multiclass/regression | Native metrics and example input; Python labeled-output scoring | Real datasets, model prediction adapters, uncertainty and calibration slices |
| Software-development tasks | Official SWE-bench route documented; external result import | Sandboxed official execution adapter |
| SAT/GRE/GMAT/civil service | Generic exam exact-match scoring | Licensed question manifests and inference harness; no invented official scores |
| Bias/hallucination/tools/conversation | Group diagnostic and labeled exact match; judged RAG aggregation | Statistical bias audits, calibrated factuality judge, actual tool execution traces |
| RAG | Supplied answer/context relevance and faithfulness judgments | Automatic grounded judgment and retrieval traces |
| BLEU/ROUGE/G-Eval | BLEU/ROUGE via optional dependencies; G-Eval result import | Calibrated local judge runner; production dataset aggregation |
| LSE/CE/DPO/PPO/flow/spectral/contrastive | Research loss functions; text+flow+alignment used in joint trainer; native flow gradient | Full preference/RL rollout system; waveform decoder spectral integration; SDE objective |
| Video/music/text datasets | Source catalog; bounded HF text helper and native text preparation | Media acquisition, normalization, codecs, cached latent shards and aligned media batch bridge |
| Shared multimodal transformer | Executable research reference and inference | Native optimized backbone, actual trained media generation |
| Rust/Protobuf/FlatBuffers | Rust active; framed Sample Protobuf active; native NPY text export active; latent FlatBuffers schema proposed | Generated verified FlatBuffers readers and mmap paths |
| Async prefetch/pinned memory | Bounded Rust queue; CUDA PinnedSlot source | Integrated native GPU consumer and measured overlap |
| YAML CPU cluster | Local placement planner | Network coordination, worker auth/health and dynamic balancing |
| CUDA attention/FA3/cuBLAS | Not implemented | Backend selection, full forward/backward and hardware-specific tests |
| Flow backward | C++ finite-difference verified; CUDA primitive source | GPU compile/parity/throughput and chain through full backbone |
| Spectral audio loss | Differentiable PyTorch multi-resolution STFT | Native FFT gradient and decoder coupling |
| NCCL | Not implemented | Collective orchestration and failure recovery |
| Checkpoint metadata | Schema; exact research CPU resume | Native complete checkpoint I/O and distributed restore |
| VBench/MM-Vet/FAD | Source catalog, provenance importer, embedding Fréchet math | Official engines running on real generated media |

Status is intentionally explicit: interfaces and catalogs do not count as runnable integrations.

## Additions after the initial foundation

- 42-profile extensible code registry, inventory/source hashes, bounded trusted-local lint/check execution, explicit unavailable/timeout/failure states and push/PR quality checks. Only the CI-selected installed toolchains are verified.
- Versioned per-shard manifests and `validate-dataset` hash/size/count/split checks.
- Evidence-derived gate measurements via JSON pointers; separately supplied scores cannot override artifacts.
- Experiment receipts/comparison with confound detection and binomial Wilson intervals.
- Lease heartbeat renewal and durable cancellation; remote control, retry limits and process cancellation are still pending.
- Small real-source reference experiment with untouched validation loss, window-bootstrap intervals, source hashes, whole-document near-duplicate grouping and exact checkpoint resume. It is not an external coding benchmark or scalable loader.

## Operator v2 follow-up

Implemented: exact code-byte preservation; explicit prose transforms; strict checkpoint computation identity; evaluator/decoding identity comparison; duplicate-group uncertainty; frozen reference CI; native run DAG, retries, concurrency, local controls, log feeds, repair proposals/apply/rollback, API/cockpit; source-backed graph catalogue with runtime projections, snapshots, diff/impact and chain/projection verification.

Current boundaries: trusted Unix workspaces, local SQLite state, 42 command profiles rather than 42 certified toolchains, live metrics explicitly unsealed, and engineering reference gates rather than frontier capability certification. No distributed worker fleet, untrusted-code sandbox, interactive DAP debugger, external notification delivery, native full GPU trainer or production multimodal codecs are implied.
