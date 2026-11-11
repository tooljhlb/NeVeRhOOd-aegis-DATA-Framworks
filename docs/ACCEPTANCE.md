# AEGIS acceptance targets

“10/10” is an aspiration, not a measured product score. Release readiness requires evidence for the intended deployment. Passing engineering fixtures does not establish model capability, universal language support or commercial value.

| Area | Required evidence before calling it complete |
|---|---|
| Recovery | Controller/worker SIGKILL before checkpoint, during atomic save, after output write and before receipt commit; power-loss simulation; corrupt checkpoint rejection; bounded attempts; no silent duplicate external side effects |
| Cache correctness | Recheck plan, declared inputs, executable and outputs on every reuse; bind transitive dependencies and environment/toolchain identities; detect undeclared dependency limits |
| Worker execution | Separate machine runs pinned source; artifact transfer verified; worker loss, stale result, duplicate delivery, expired lease and cancellation tested; worker trust and attestation stated |
| Isolation | Untrusted workloads use tested container/VM isolation with network, filesystem, CPU, memory and GPU policy; credentials scoped per task; escape and secret-exposure tests |
| Scale | Published throughput/latency/memory at 1, 10, 100 and 1,000 concurrent jobs; backpressure; fair scheduling; durable queues; admission and spend limits |
| Data | Streaming bounded-memory shards, provenance/license policy, exact/near duplicate and benchmark contamination tests, deterministic exact sampler resume |
| AI evaluation | Frozen independent corpora, executable coding tasks, task-specific metrics, group uncertainty, paired comparisons and independently reproducible evidence |
| Languages | Each claimed language has an installed toolchain and actual lint/build/test evidence; registry entries alone are insufficient |
| Training | Native GPU forward/backward/optimizer equivalence, finite-gradient checks, interruption recovery, measured memory/throughput; equal-budget architecture comparisons |
| Multimodal | Real versioned codecs with playable round trips, alignment/time masks, actual audio/video benchmarks and blind human comparisons |
| Repair | Reproduce failure, isolated patch, relevant semantic/regression tests, evidence gate, rollback; formatting success alone cannot approve semantic changes |
| Developer experience | Fresh-machine install and first useful run, actionable errors, accessible cockpit, searchable logs, debugging integrations and API/CLI compatibility |
| Teams | Authentication/authorization, tenant isolation, audit and retention policies, backup/restore drills, dependency inventory and vulnerability response |
| Product value | Independent users complete representative real projects with measured reductions in effort/failures; deployment and support costs recorded |

## Current milestone

The local operator has passed concurrency, policy, cancellation, repair rollback, graph integrity and actual controller-SIGKILL integration checks. Completed-run reuse now rejects changed declared inputs and executable bytes. Recovery omits resume arguments when no checkpoint exists. Existing checkpoints are validated by the reference trainer before state is loaded; corrupt or incompatible checkpoints fail closed. Arbitrary external programs must implement their own equivalent validation contract.

The separate-worker workflow uses distinct GitHub-hosted jobs for the controller, worker and receiving verifier. The controller pins a commit; the worker runs the complete 11-stage CPU workflow; the receiving machine verifies exported file hashes and evaluated checkpoint identity. This is a trusted CI worker adapter, not a general remote fleet, attested compute or a universal exactly-once side-effect guarantee. Repository write access is part of the trust boundary.

## Delivery order

1. Recovery and identity correctness, crash fault injection, separate-worker round trip.
2. Durable worker leases/heartbeats, duplicate and stale-return rejection, cancellation and disconnected recovery.
3. OS workload isolation, resource admission and credential scoping.
4. Streaming datasets, exact sampling recovery and independently maintained executable benchmarks.
5. Native GPU evidence and tested audio/video codecs.
6. Scale measurements, team operation, packaging, external pilot users and measured product value.

Each milestone requires executable checks and published receipts. Unavailable hardware or independent users remain explicit evidence gaps; they cannot be replaced by an internal rating.

## Research basis

Temporal's activity documentation distinguishes retryable activity execution from side-effect idempotency: retries require deliberate application contracts. AEGIS likewise does not promise that arbitrary commands are safe to replay. See https://docs.temporal.io/nexus/operations and https://docs.temporal.io/tasks . OpenSSH host verification is required for a future SSH transport; disabling StrictHostKeyChecking is not a supported shortcut: https://man.openbsd.org/ssh_config.5 .

## Worker/isolation milestone

Durable **local shared-volume** workers now connect the lease ledger to actual operator execution, with heartbeat cancellation, fresh epoch workspaces, immutable bounded bundles, fenced sealed results and worker-death tests. Optional digest-pinned OCI stages enforce CPU/memory/PID and network/filesystem limits when Linux Docker is present. This closes part of milestones 2 and 3. A remote lease transport, VM isolation, disk quotas, external orphan reaping, multi-tenant auth and large-scale measurements remain open; the shared SQLite WAL queue must not be put on a network filesystem to simulate a distributed controller.

## Native remote transport and acceptance registry

The Rust remote controller/client now provides distinct worker authentication, controller-owned lease time, bounded chunk transfers, fenced output publication and byte verification without sharing controller storage. The transport integration tests disconnect and restart the controller, recover into a new epoch, cancel live remote work and exercise malformed/stale returns. Production TLS provisioning, remote fleet capability placement, large checkpoint transfers and independently provisioned host certification remain acceptance gaps.

`readiness.json` and the native `readiness` command track 20 areas / 84 declared checks. They expose partial and unverified work and verify attached evidence hashes. This is acceptance coverage, not a replacement for independent product or model benchmarks. See [the registry contract](READINESS.md).
