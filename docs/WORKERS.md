# Durable workers and isolated stages

## Shared-volume worker v1

The Rust worker executes one queued operator plan per invocation. The controller owns a SQLite WAL ledger and spool on a **local, trusted filesystem**. Multiple worker processes on that host may claim jobs. SQLite WAL is not a supported multi-host network-filesystem queue. The separate GitHub worker workflow remains the cross-machine transport proof; it does not yet use this local queue as an internet service.

```sh
# bundle/ contains only the source/assets needed by the plan, no generated output.
aegis-data worker-enqueue /var/lib/aegis/jobs.db plan.json bundle/
aegis-data worker-once /var/lib/aegis/jobs.db worker-01 --ttl 15 --trusted-local
aegis-data worker-status /var/lib/aegis/jobs.db my-job
aegis-data worker-verify /var/lib/aegis/jobs.db my-job
# Existing cancellation contract applies to queued and active work:
aegis-data cancel /var/lib/aegis/jobs.db my-job
```

`worker-once` returns `idle` when no eligible work exists. A service manager may invoke it repeatedly. Bundle snapshots permit at most 256 MiB / 10,000 regular files and reject symlinks and special files. Submit a deliberately prepared source bundle; there is no implicit crawl of private directories. The submitted `workspace` is replaced with an epoch-specific copy. Programs/toolchains outside the bundle must already exist on the worker. This bounded v1 snapshot is not the streaming large-dataset loader.

Each claim advances a fencing epoch. Heartbeats run at roughly one quarter of the lease TTL (3–300 seconds). Expired claims may be recovered up to three total claims; exhausted jobs become failed during the next claim scan. An explicit execution/validation failure is terminal, avoiding blind retries of deterministic failures. Worker death recovers into a fresh epoch workspace from the immutable bundle; it restarts the plan rather than automatically trusting the dead worker's mutable checkpoint. Within a surviving operator attempt, the existing validated checkpoint resume contract applies.

Renewal and completion require the current owner, epoch, live expiry and leased state. Cancellation invalidates renewal. Workers stop their operator child on failed renewal and cannot publish an accepted result. Old epochs keep separate workspaces and receipts; only the ledger's current committed result is accepted. Returned output hashes and the event graph are rechecked by `worker-verify`. These hashes protect integrity relative to trusted controller storage, not against a malicious host able to rewrite the ledger and spool. Protect that directory with OS permissions.

Local worker tests kill a real worker, wait for lease expiry and run a replacement; check heartbeats beyond the original expiry; reject stale completions, modified bundles and returned output; and cancel active work. Full power-loss durability, remote authentication, multi-tenant storage, distributed clock authority and a remote lease API remain future work.

## Optional OCI isolation

A stage may declare:

```json
{
  "id": "test",
  "operation": "test",
  "program": "/bin/sh",
  "args": ["-c", "your-test-command"],
  "outputs": ["result.txt"],
  "container": {
    "image": "your-image@sha256:REPLACE_WITH_64_HEX_DIGEST",
    "memory_mib": 512,
    "cpus": 1,
    "pids": 64
  }
}
```

The example digest is a placeholder and will be rejected until replaced. Provision the image explicitly before running; execution uses `--pull=never`. The program path is resolved inside the image. No fallback to host execution is permitted when Docker, the pinned image or enforcement is unavailable. Omission of `container` preserves explicitly trusted host execution; this is not an automatic untrusted-workload classifier.

Linux Docker mode uses the local `/var/run/docker.sock` daemon, digest-pinned images, no external network, a read-only container root filesystem, all capabilities dropped, no-new-privileges, current host UID/GID, bounded CPU/memory/swap/process count, and a 64 MiB temporary filesystem. Only the chosen workspace is bind-mounted read/write; put only permitted data there. The daemon socket and host credentials are not mounted. Logs, outputs and the host workspace still need disk quotas; this version does not enforce a total disk-write budget or GPU isolation. Containers share the host kernel, so this is not a VM boundary or escape-resistance certification.

Cancellation/timeout cleans up the deterministic AEGIS container name. A restarted operator reconciles that name before execution; a replacement queue worker cleans containers from older epochs. If the controller and worker both die, a daemon-managed container may survive until recovery reconciliation. Deploy an external reaper before relying on unattended multi-tenant operation. Docker control calls have ten-second deadlines. Direct host execution cannot provide the same descendant/resource containment.

CI's isolation job performs actual OCI execution, checks cgroup CPU/memory/PID settings, absent external network/daemon socket, read-only root and writable declared workspace, and rejects unpinned images. Local environments without Docker cannot validate those kernel guarantees. Use the uploaded isolation receipt and job outcome for that evidence.

## Remote transport extension

The [remote controller/client](REMOTE.md) now transports plans and files through authenticated sockets without a shared worker filesystem. The SQLite database stays on the controller. This does not change the local shared-volume queue's storage requirements. Use a dedicated queue for remote jobs and `remote-verify` for their returned bytes.
