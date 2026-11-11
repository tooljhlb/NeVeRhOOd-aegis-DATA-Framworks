# Security

This application can process source code, execute commands and transfer artifacts. Treat run plans and adapters as potentially powerful operations.

- Use `--trusted-local` only for workspaces you control.
- Run untrusted/generated commands inside reviewed, digest-pinned OCI containers.
- Keep the control API bound to loopback or behind a correctly authenticated gateway.
- Store worker authentication and service secrets outside the repository.
- Inspect exported evidence and logs before sharing them; they can include source or environment information.
- The project does not claim certified sandboxing, multi-tenant isolation, or production-ready cloud hosting.

**Report a vulnerability privately** to the maintainer through available GitHub contact or private vulnerability-reporting controls. Do not open a public issue containing an active exploit, credentials or sensitive information.

See [remote-worker boundaries](docs/REMOTE.md), [runtime profiles](docs/RUNTIME.md), and [artifact transport](docs/ARTIFACTS.md).
