# Contributing

Thanks for helping build AEGIS DATA Frameworks.

This is a working Rust-native developer framework; improvements should come with a reproducible test, explicit failure behavior, and documentation that distinguishes verified implementation from planned work.

## Before opening a pull request

1. Open an issue for breaking changes, public API revisions, new runtimes or major subsystems.
2. Keep contributions provider-independent and avoid requiring external model keys.
3. Run `cargo test --workspace --locked`. For C++ tests, run `bash scripts/verify.sh`.
4. Never commit credentials, private dataset records, model weights, cloud configurations, tokens or personal filesystem paths.
5. Include exact reproduction instructions, the expected vs actual result, and a small public fixture where possible.
6. Preserve provenance and keep failures/unverified gates explicit. A hash only proves identity, not capability.

## Architecture principles

- Rust is the native control/data plane; Protobuf is for binary records and contracts.
- JSON may be used for configuration, control and exported results.
- Python/PyTorch is an optional research reference, not a mandatory runtime for the native CLI.
- Tests and security boundaries precede new provider integrations or automated execution privileges.
- Changes to limits, timeouts or trust policies require accompanying tests.

See [the roadmap](docs/ROADMAP.md) and [capabilities](docs/CAPABILITIES.md).
