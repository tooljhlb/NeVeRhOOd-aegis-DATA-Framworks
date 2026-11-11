# Runtime execution profiles

CLI and HTTP operators share native per-run admission through `runtime::Profile`.
Profiles constrain exact program names, digest-pinned container images, slots,
wall time, stage timeouts, retries, and a conservative concurrent container
CPU/memory envelope. Unknown JSON fields are rejected.

```sh
aegis-data runtime-check profiles/isolated.json plan.json
aegis-data runtime-run profiles/isolated.json plan.json operator.db --trusted-local
AEGIS_RUNTIME_PROFILE=profiles/isolated.json aegis-data operator-serve operator.db . --trusted-local
```

Use `operator-serve --help` for address options. The API loads and validates its
profile once at startup and applies it before accepting every POST `/v1/runs`.
An invalid configured profile prevents startup. Changes require restart.
Without the environment variable, the existing trusted-local API retains its
previous behavior. Profiles are administrator configuration, not caller input.

Example profile (replace the image digest with the actual locally available image):

```json
{
  "schema_version": 1,
  "id": "isolated-small",
  "require_container": true,
  "allowed_programs": ["sh"],
  "allowed_images": ["busybox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
  "max_slots": 2,
  "max_wall_seconds": 600,
  "max_attempts": 2,
  "max_stage_seconds": 120,
  "max_cpu": 2,
  "max_memory_mib": 512
}
```

Admission does not execute a program. `runtime-run` admits, then delegates to the
existing durable operator, retaining checkpoints, logs, input/output checks,
retry rules, and cancellation. The trust flag remains explicit; a profile does
not convert host execution into a sandbox. Container execution retains the
existing network-disabled, pinned-image isolation policy.

The envelope sums the largest resource allocations up to the plan's slot count.
This intentionally overestimates where dependencies prohibit overlap. It is
per run, not a machine-wide reservation or a team quota. The API can accept two
runs concurrently, so shared-host capacity must account for both. Host programs
have no enforced CPU/memory limit; require containers for bounded workloads.

External schedulers can call the authenticated API through an authenticated
gateway or verified tunnel. This change does not open a public listener, add
provider credentials, provision machines, or impose profiles on separately
invoked worker controllers. Workers retain their own existing isolation policy.

## Capacity shared across API runs

Set `AEGIS_API_CAPACITY=/path/capacity.json` alongside a container-only
`AEGIS_RUNTIME_PROFILE`. Capacity JSON contains `max_runs`, `cpu`, and
`memory_mib`, for example `{"max_runs":2,"cpu":4,"memory_mib":4096}`.
Invalid configuration or a profile permitting host execution prevents startup.

The server atomically reserves each admitted plan's conservative container
resource envelope before submitting it. Exhausted capacity returns HTTP 429
without launching the run. Reservations are released when the operator future
exits, including failure or cancellation. GET `/v1/runtime` exposes configured
limits and current reservations. The existing two-active-run limit still applies.

This pool coordinates one API process, not other CLI commands, API processes,
remote controllers or unrelated host workloads. It is not a machine-wide quota,
a durable queue or distributed scheduler. Separate worker hosts and controllers
need their own admission configuration. A server crash loses in-memory
reservations; existing process/container recovery limitations still apply.
