# Public developer-workspace acceptance loop

The product serves ordinary software repositories as well as AI/model projects.
This is a staged acceptance plan; a working operator is not yet a full IDE.
Testing occurs throughout development rather than after assigning a 10/10 score.

| Goal | Implemented foundation | Remaining acceptance work |
|---|---|---|
| Automatic setup | Bounded repository discovery, language inventory, nested manifest roots, generated CLI/API plans and profiles, missing executable reporting, cockpit review control | More build systems, dependency/version checks, environment provisioning, independent repository matrix |
| Editor workspace | Operator cockpit plus plain-text source tree/tabs/search, revision saves/snapshots, Git review, bounded command panel and optional local Monaco with saved-file LSP markers and backend diagnostics/navigation | Persistent monaco-languageclient integration, indexed search, completion, DAP sessions, interactive terminal, persistent buffers and browser/desktop usability validation |
| Durable agent tasks | Durable operator stages, logs, cancellation, leases and recovery; executable proposal adapters and local model SSE contract with disconnect cancellation | Persistent agent objective/context/tool state, UI-reload continuation, reconnect and qualified agent permissions |
| Change management | Protected tests, repair rehearsals, preimage checks, editor stale-save rejection/snapshots and bounded single-file promotion/rollback | Multi-file atomic changes, overlapping-edit detection, per-task worktrees and concurrent human/agent transactions |
| Consistent interfaces | Shared run IDs and native contracts; CLI inspection of API runs; CLI/API/UI project discovery, local LSP and model streaming contracts | Broader route coverage, version negotiation, typed external client SDKs, streaming event transport |
| Public installation | Verified Linux x86_64 runtime package and fresh-install project workflow | Independently operated installs, Windows/macOS, versioned migrations, signed distribution and update recovery |

## Discover a project without writing plan JSON

```sh
aegis-data project-discover /path/to/repository /tmp/aegis-discovered --run-id unique-project-check-001
# Review discovery.json, command arguments, input coverage and host execution policy.
aegis-data project-run /tmp/aegis-discovered/profile.json /tmp/aegis-discovered/plan.json /tmp/project.db /tmp/project-export --trusted-local
```

The destination directory must be new. Keep generated configuration outside the
source workspace so that future discovery does not treat it as an input.
Discovery reads manifests; it executes no commands and installs no dependencies.
It proposes Cargo test, Go test, npm build/test scripts, pytest for pyproject
projects with tests/, and CMake configure/build/CTest. Unsupported repositories
produce no guessed success. Missing executables are reported; package/module
availability, driver compatibility and complete dependency resolution are not
certified. npm test scripts can require project-specific CI/watch options: review
and adjust arguments before execution.

The bounded scan descends at most eight levels, uses ignore rules, avoids symlink
traversal and common generated directories, limits source files to 1 MiB each,
20,000 visited directory entries, 10,000 inspected files and 64 MiB of included inputs. Generated plans are also
bounded below the API request limit; oversized multi-project plans require a
smaller selected project root. Common .env/private-key
files are excluded; this is not a complete secret scanner. Review all declared
inputs before exporting. Large files and skipped paths are outside identity
coverage. Cargo [workspace] roots suppress nested Cargo manifests; unusual TOML
or package-manager configurations need explicit review.

Generated plans do not declare backend-specific output artifacts. Use a fresh
run ID for each execution; add explicit output paths to enable stronger completed
artifact validation and reuse. Generated host profiles allow detected command
names but impose no OS-level host resource limits. Container-only server policies
will reject these host plans until reviewed pinned container settings are added.

Authenticated POST `/v1/projects/discover` accepts
`{"workspace":"/path/to/repository","run_id":"unique-id"}` and returns the same
plan/profile/report contract as the CLI. Paths must remain inside the API server
root. The cockpit's Detect project button fills the plan review field without
submitting it; Submit plan still starts the existing durable run API. Discovery
has no separate task identity: the generated run ID is the one used by status,
logs, events and cancellation across interfaces.

## Architecture decisions

Start with the trusted local operator and desktop/workspace client. A public
hosted service needs separate tenant identity, scoped auth, secret management,
worker isolation, shared quotas and billing boundaries; a loopback token is not
that design. Keep both clients on versioned native task contracts.

Model independence uses explicit adapters with declared capabilities, protocol
version, health, cancellation, weight/processor compatibility and evidence. The
existing executable prediction/proposal adapters are foundations, not universal
streaming inference adapters.

Use LSP for language features and DAP for debugging. Decide extension APIs before
advertising compatibility; a language registry does not implement VS Code's
extension host, Marketplace access or arbitrary VSIX execution. No extension
compatibility is claimed yet.

Add independent repositories spanning toolchains and task types. Measure actual
task completion, follow-up regressions, first-success/recovery time and human
interventions using fixed definitions. Current automated discovery evidence uses
this repository's real C++ component and synthetic detection tests; independent
project/user outcomes remain unmeasured.
