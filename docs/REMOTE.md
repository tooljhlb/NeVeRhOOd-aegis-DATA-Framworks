# Authenticated remote workers

This transport connects native workers to a controller **without sharing the SQLite database or spool**. Workers receive an immutable plan and hashed input files over HTTP, execute in their own epoch directory, and upload declared outputs. The controller rehashes the uploaded bytes and checks lease ownership, expiry, epoch and output membership before committing the return.

## Start a controller

Create a private token registry on the controller. Generate a distinct random secret of at least 32 characters per worker using your secret manager. Example shape (replace the placeholder):

```json
{"worker-01":"REPLACE_WITH_A_RANDOM_SECRET_AT_LEAST_32_CHARACTERS"}
```

```sh
chmod 600 worker-tokens.json
aegis-data worker-enqueue /var/lib/aegis/remote.db plan.json bundle/
aegis-data worker-controller /var/lib/aegis/remote.db \
  --tokens worker-tokens.json --bind 127.0.0.1:8790 --ttl 30
```

Use a dedicated remote queue. Keep the controller database and spool on its local filesystem. Registry tokens authenticate workers only; they do not grant HTTP enqueue, administrative cancellation or arbitrary filesystem access. Queue submission and cancellation remain controller CLI operations. All workers in this v1 registry can claim any queued job; project-scoped RBAC and capability-aware placement are not implemented.

The server deliberately binds loopback. For another machine, use a verified SSH tunnel or authenticated TLS reverse proxy. For example, after configuring and verifying the host key:

```sh
ssh -N -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -L 8790:127.0.0.1:8790 operator@controller-host
```

The example names are placeholders, not provisioned resources. Do not disable host key verification. Never expose this HTTP listener directly on a public interface.

## Run a worker

Install the same AEGIS release and required runtime/toolchains on the worker. Supply its token through a protected environment or secret manager, not a URL or CLI token argument:

```sh
# AEGIS_WORKER_TOKEN is supplied by your secret manager.
aegis-data worker-remote http://127.0.0.1:8790 worker-01 /var/lib/aegis/attempts
```

Each invocation handles one job or returns `idle`. Remote execution defaults to requiring `container` on every stage. `--trusted-local` explicitly permits host commands for controlled development; it is not required for isolated plans. A pinned-image Docker runtime must be installed and the image provisioned on the worker for OCI stages. A host-native executable path from the controller may not exist on another machine: plans must use installed tool names, portable scripts through explicit interpreters, or image entrypoints. Downloaded bundle files do not preserve executable mode bits; invoke source scripts through their interpreter.

The client accepts HTTPS endpoints or loopback HTTP (for tunnels), validates TLS normally, disables redirects and environment proxies, and applies connection/request deadlines. It does not offer an insecure TLS bypass. The worker token is stripped from the environment of the workload supervisor. Provision a separate worker token per machine; rotate/revoke registry entries by restarting the controller with an updated private registry. Dynamic token reload, OIDC and mTLS are not implemented.

## Transfer and recovery contract

- Controller wall time determines lease expiry. TTL is 6–300 seconds; the client renews at roughly one quarter of TTL.
- Input and return chunks are at most 1 MiB. Input snapshots and each epoch's returned files are bounded to 256 MiB / 10,000 files. This is not a large-checkpoint transport yet.
- Paths must match the declared input/output names and remain relative. Unknown paths, traversal, wrong offsets, conflicting retransmissions and oversized chunks are rejected.
- Identical uploaded chunks can be retransmitted. An identical completion can be acknowledged twice; a conflicting completed return cannot replace the committed result.
- Every remote epoch gets a create-new worker directory and a fresh immutable source copy. Interrupted remote work restarts the plan; cross-worker checkpoint transfer/resume is not implemented.
- A failed renewal (including cancellation, expired ownership or an unavailable controller) stops the local operator rather than allowing result promotion. A lost process's lease can later be reclaimed, with a new epoch and separate output directories.
- Cancelling through `aegis-data cancel DB JOB` invalidates remote renewal. The client stops its local worker; the previous local-worker container cleanup rules apply. Unreachable/dead machines still require an external orphan reaper before unattended multi-tenant deployment.

```sh
aegis-data worker-status /var/lib/aegis/remote.db my-job
aegis-data remote-verify /var/lib/aegis/remote.db my-job
aegis-data cancel /var/lib/aegis/remote.db my-job
```

The controller marks **transport integrity**, not independently attested execution or model quality. A trusted worker can assert completion; hashes prove that the controller received the identified bytes. Model acceptance continues to require the operator's evaluation/gate evidence. These endpoints do not turn a malicious worker into trusted compute.

## Verification and remaining boundaries

`scripts/verify_remote.py` exercises the actual Rust server/client through sockets with separate controller and worker directories: multi-chunk transfer, authentication, lease reassignment, cross-worker denial, stale upload rejection, duplicate/conflicting retransmissions, body limits, traversal rejection, hash mismatch, post-return tamper, controller disconnect/restart and active cancellation. It is a multi-process transport integration test on one host. The existing separate-GitHub-job workflow remains an independent cross-machine artifact test. A newly provisioned remote host using this lease API has not been certified by those two tests alone.

This release does not include a public control plane, user/team authorization, worker capability placement, remote live logs, VM isolation, disk quotas, large checkpoint transport, fleet autoscaling or load-tested service-level objectives. The synchronous controller filesystem operations and single transfer lock intentionally favor simple correctness at this bounded scale. Measure and redesign those paths before large fleets.

Research references: reqwest request timeouts/redirect policy https://docs.rs/reqwest/0.12/reqwest/blocking/struct.ClientBuilder.html ; Axum request-body enforcement https://docs.rs/axum/0.8/axum/extract/struct.DefaultBodyLimit.html .
