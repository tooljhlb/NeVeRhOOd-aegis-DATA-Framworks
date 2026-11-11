# AEGIS local operator v1

## Runnable workflow

From the cloned repository, install the isolated research dependencies and use the native CLI:

```bash
pip install -e './research[eval]'
cargo build --locked
cargo run --locked -- operator-reference /tmp/operator.db fixtures/reference-code --out runs/reference-v2 --run-id reference-v2 --steps 20 --python python --trusted-local
cargo run --locked -- operator-status /tmp/operator.db reference-v2
cargo run --locked -- operator-events /tmp/operator.db reference-v2 --after 0
cargo run --locked -- graph-verify /tmp/operator.db
```

This builds and executes a versioned dependency plan: source preparation → shard validation → initialization baseline → training and an independently resumed branch → checkpoint equality proof → held-out evaluation → comparison → acceptance gates → local artifact export. Every stage has one run ID, status, attempts, logs and receipts. The acceptance rule is a small reference-pilot nonregression and exact-CPU-resume rule; it is not a claim of frontier ability or an official coding benchmark.

Use `--plan-only` to emit the plan for review or API submission. Use a new run ID/output directory for a new experiment. Existing run IDs reject different plans. A completed run checks its declared artifact hashes before returning a cached result. Interrupted stages use their explicit retry budget and optional resume arguments; arbitrary applications do not acquire checkpoint support just by being launched here.

## Controls and execution modes

```bash
cargo run --locked -- operator-control /tmp/operator.db reference-v2 pause
cargo run --locked -- operator-control /tmp/operator.db reference-v2 cancel
cargo run --locked -- operator-note /tmp/operator.db reference-v2 'Inspect the validation regression before another run' --actor human
```

Pause takes effect at a stage boundary: running stages finish, new stages stop launching. Cancel requests terminate active process groups. Resume requires setting control to `run`, then resubmitting the identical saved plan with `operator-run DB PLAN.json --trusted-local` or through the API. The reference command regenerates the same plan when invoked with identical arguments. Reuse is conditional on declared input/output hashes; stages without reusable outputs may need to rerun within their attempt budget. Undeclared inputs cannot be verified. Prepare failures that leave a partial output directory may require a fresh run rather than a blind retry.

`observe` and `assist` record a plan/proposal without executing it. `bounded` executes only operation classes explicitly allowed by the plan. `autonomous_workspace` uses the same explicit operation/budget checks in a user-supplied trusted workspace. Neither mode provisions an OS sandbox. They do not authorize production deployment, external messaging, secret discovery, or arbitrary environment mutation on their own.

Concurrency uses up to 16 logical slots, stage costs and dependency readiness. Slots are scheduling units, not measured CPU, GPU-memory or financial limits. The wall-time limit applies to a controller invocation; per-stage attempt counters persist across recovery. Explicit retryable exit codes, bounded attempts and backoff prevent blind restart loops. The reference trainer also records measured steps/sec, text-target tokens/sec, host peak RSS on Linux, and GPU allocated peak only when actually running CUDA. Timing scope is recorded; no GPU throughput is inferred from source code.

Only one local controller can own a run through an advisory OS lock. SQLite WAL coordinates local state and multiple local runs; network/distributed execution, multi-tenant roles, signed cross-host receipts, fleet scheduling and extreme-scale storage backends remain future integrations.

## API and cockpit

Set a long random `AEGIS_OPERATOR_TOKEN` in the process environment, then run:

```bash
cargo run --locked -- operator-serve /tmp/operator.db . --bind 127.0.0.1:8787 --trusted-local
```

The service binds loopback only. Open the printed URL for the embedded HTML cockpit. Its session token remains in page memory. It shows run status, event/decision feeds, graph topology, measured loss curves and live stdout/stderr. Pause, cancel, resubmit and operator-note controls use the same authenticated API as an AI client. Actors are caller-supplied audit labels, not independently authenticated identities.

| Endpoint | Contract |
|---|---|
| `GET /health` | Public service health only |
| `GET /` | Static cockpit; no private run data without authentication |
| `POST /v1/runs` | Bearer-authenticated plan submission under the server workspace root |
| `GET /v1/runs/{id}` | Saved plan, status, stage attempts and receipts |
| `GET /v1/runs/{id}/events?after=N` | At most 1,000 ordered events per cursor page |
| `POST /v1/runs/{id}/control` | `{action: pause/cancel/run, actor: ...}` |
| `POST /v1/runs/{id}/notes` | Append operator context without silently changing execution |
| `GET /v1/runs/{id}/logs/{stage}/{attempt}?stream=stdout&offset=N` | Bounded byte-offset log feed |
| `GET /v1/runs/{id}/metrics` | Declared live metric files; explicitly unsealed telemetry |
| `GET /v1/graph` | Versioned recorded graph projection |

Use `Authorization: Bearer TOKEN` for `/v1` requests. External deployment needs a separately configured authenticated gateway and stronger tenancy controls. There is no automatic connection to another AEGIS runtime, teacher model, cloud account, or production worker.

## Rejection and notifications

Rejections record the failed stage, reason, applicable policy, and choices such as inspecting logs, correcting inputs/plan, or creating a new run. Missing artifacts or false gate results cannot be overridden by a favorable free-standing score. Notifications are durable local events, displayed in the cockpit/API. No email, Slack or other person-directed notification is sent.

## Isolated repair proposals

```bash
cargo run --locked -- repair-propose path/to/file.py /tmp/repair-proposal --tool ruff-format
cargo run --locked -- repair-apply path/to/file.py /tmp/repair-proposal/proposal.json
cargo run --locked -- repair-apply path/to/file.py /tmp/repair-proposal/proposal.json --rollback
```

Supported proposal tools are `ruff-format`, `ruff-fix`, and `clang-format`. They modify an isolated candidate copy; proposal generation leaves the source untouched. Receipts bind original/candidate hashes and tool/check output. Apply rejects changed sources or candidate bytes, preserves file permissions, and publishes via rename. Rollback requires the expected candidate still be present. Candidate lint success is not semantic correctness; project tests are required before configuring automatic promotion. Source replacement is a trusted local operation and does not replace coordination with other editors/processes.

Debugger executables can be configured as bounded stages (for example a prepared GDB batch job), with logs and receipts. Interactive debugger/DAP integration, general AI-generated semantic repairs, container execution, signed approvals, remote notifications and distributed scheduling are not claimed as complete.

## Crash recovery and remote verification

Completed run reuse verifies declared input, executable and output hashes. A changed identity requires a new run. Retry `resume_args` supports `--resume PATH`: a missing checkpoint restarts without resume arguments, an empty checkpoint is rejected, and a present checkpoint must pass the executing trainer's validation. The reference trainer validates computation, data and optimizer identity before loading training state. External programs must supply equivalent validation. Attempts remain bounded after controller death.

`scripts/verify_operator.py` kills actual controller processes before and after a checkpoint marker, then restarts the same persisted plan. These transport/control fixtures complement actual trainer identity and exact-resume checks; the marker fixture is not a neural checkpoint validation test.

`.github/workflows/remote-worker.yml` runs the full reference workflow on a separate worker job and returns artifacts to another job for verification. Run it from Actions → Separate worker artifact round trip → Run workflow. It also executes on push. Returned exports contain an explicit source revision and manifest digest; `scripts/verify_worker_return.py` checks the revision against the controller's pinned commit, artifact membership/hashes, acceptance and evaluated checkpoint binding. The CI worker is trusted; these hashes do not attest a malicious worker. See [acceptance targets](ACCEPTANCE.md) for the remaining release criteria.

For durable worker claims, heartbeat cancellation, fenced output verification and optional OCI stage limits, see [workers and isolation](WORKERS.md). Host execution remains explicitly trusted; isolated stages must opt in with a digest-pinned image.

The [authenticated remote worker guide](REMOTE.md) extends local leasing to socket-based plan/input delivery, fenced uploads and controller-verified returns. This is a separate loopback service designed for a TLS gateway or verified SSH tunnel; remote stages require isolation unless host execution is explicitly trusted.
